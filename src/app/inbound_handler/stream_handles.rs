//! Handles and per-chunk data shared by the streaming dispatch modules.
//!
//! The connection loop, the dispatch slot, and the parking queue each need the
//! same mutable connection handles to answer a streaming handler, plus the
//! read timeout that bounds teardown. Grouping them here keeps every helper
//! within the repository's argument limit, and makes it explicit that a reply
//! travels the same pipeline whichever path produced it.
//!
//! The bundled handles deliberately exclude the inbound reader. Reading the
//! next frame borrows the whole inbound framing state, so a bundle that held
//! the reader could not coexist with it. Each action therefore builds the
//! bundle from disjoint borrows for the duration of that action only.

use std::{io, time::Duration};

use bytes::Bytes;
use futures::Sink;

use super::{
    super::{
        codec_driver::FramePipeline,
        envelope::{Envelope, Packet},
        frame_handling,
    },
    stream_dispatch::DispatchContext,
    stream_parked::{self, ParkedMessage, ParkedQueue, parked_bytes},
    stream_slot::ConnectionStreams,
};
use crate::{
    codec::FrameCodec,
    connection::ConnectionActor,
    message_assembler::{MessageAssemblyState, MessageKey, StreamingAssemblyId},
    request::{QueuedBytes, RequestParts},
    serializer::Serializer,
};

#[cfg(test)]
#[path = "stream_handles_tests.rs"]
mod tests;

/// Mutable connection handles needed to answer a streaming handler.
///
/// Borrow them for one call with [`DispatchHandles::reborrow`] when the caller
/// must keep using the bundle afterwards.
pub(super) struct DispatchHandles<'a, S, E, K, F>
where
    S: Serializer + Send + Sync,
    E: Packet,
    K: Sink<F::Frame, Error = io::Error> + Unpin + Send,
    F: FrameCodec,
{
    /// Connection-local assembly state.
    pub(super) assembly: &'a mut Option<MessageAssemblyState>,
    /// Outbound pipeline shared by every reply on the connection.
    pub(super) pipeline: &'a mut FramePipeline<F::Frame>,
    /// Write half the connection actor drives.
    pub(super) sink: &'a mut K,
    /// Actor that schedules and delivers every reply.
    pub(super) actor: &'a mut ConnectionActor<Envelope, (), F::Frame>,
    /// Connection-wide byte account, shared with every body on this
    /// connection so a pre-filled body is charged against the same budget as
    /// an incrementally delivered one.
    pub(super) queued: QueuedBytes,
    /// Immutable connection inputs.
    pub(super) ctx: &'a DispatchContext<'a, S, E, F>,
    /// Maximum wait for a handler or a body send while teardown runs.
    pub(super) wait: Duration,
}

impl<S, E, K, F> DispatchHandles<'_, S, E, K, F>
where
    S: Serializer + Send + Sync,
    E: Packet,
    K: Sink<F::Frame, Error = io::Error> + Unpin + Send,
    F: FrameCodec,
{
    /// Reborrow every handle for a single call, leaving this bundle usable.
    ///
    /// # Examples
    ///
    /// ```ignore
    /// let mut handles = DispatchHandles { .. };
    /// slot.close_active(None, handles.reborrow()).await?;
    /// // `handles` is still live here.
    /// ```
    pub(super) fn reborrow(&mut self) -> DispatchHandles<'_, S, E, K, F> {
        DispatchHandles {
            assembly: &mut *self.assembly,
            pipeline: &mut *self.pipeline,
            sink: &mut *self.sink,
            actor: &mut *self.actor,
            queued: self.queued.clone(),
            ctx: self.ctx,
            wait: self.wait,
        }
    }
}

/// Bytes retained elsewhere, and the aggregate limit a chunk must fit within.
#[derive(Clone, Copy, Debug)]
pub(super) struct ChunkBudget {
    /// Bytes retained by assembly, fragmentation, and parked messages.
    pub(super) other_retained: usize,
    /// Aggregate byte limit the queued body must stay within.
    pub(super) limit: usize,
}

impl ChunkBudget {
    /// Measure the budget a body chunk must fit within on this connection.
    ///
    /// Every holder outside the chunk queue counts towards `other_retained`,
    /// including messages parked behind the live body. Omitting the parked
    /// queue would let a chunk in only to trip the hard cap on the next loop
    /// step, which aborts the connection instead of holding the chunk back.
    ///
    /// One documented exception applies, and no measurement can override it:
    /// `AccountedBodySender::send_chunk` admits a single chunk into an empty
    /// account whatever that chunk's size or `other_retained`. The exception
    /// exists so a body whose first chunk alone exceeds the budget can still
    /// start, which means the aggregate limit is a strict cap only for the
    /// chunks that follow. Otherwise admission requires the queued bytes,
    /// `other_retained`, and the chunk to fit within `limit`.
    ///
    /// The measurement is a snapshot and is deliberately not kept for the life
    /// of one admission: the loop measures again before every drive, so bytes
    /// released in between — an expired assembly, a drained parked message, a
    /// handler that took its chunk — are seen. Between drives the totals can
    /// only shrink: the one frame the loop may read while an admission is
    /// pending is buffered uncharged, and nothing else it does retains bytes.
    pub(super) fn measure<S, E, K, F>(
        handles: &DispatchHandles<'_, S, E, K, F>,
        parked: &ParkedQueue,
    ) -> Self
    where
        S: Serializer + Send + Sync,
        E: Packet,
        K: Sink<F::Frame, Error = io::Error> + Unpin + Send,
        F: FrameCodec,
    {
        Self {
            other_retained: retained_outside_queue(
                handles
                    .assembly
                    .as_ref()
                    .map_or(0, MessageAssemblyState::total_buffered_bytes),
                handles.pipeline.fragmentation_buffered_bytes(),
                parked,
            ),
            limit: frame_handling::active_aggregate_limit_bytes(handles.ctx.effective_budgets),
        }
    }
}

/// Sum the bytes assembly, fragmentation, and parked messages retain.
pub(super) fn retained_outside_queue(
    assembly_bytes: usize,
    fragmentation_bytes: usize,
    parked: &ParkedQueue,
) -> usize {
    assembly_bytes
        .saturating_add(fragmentation_bytes)
        .saturating_add(parked_bytes(parked))
}

/// The first body chunk of a streaming request.
#[derive(Debug)]
pub(super) struct FirstChunk {
    /// Assembly key, absent when no assembler participates.
    pub(super) key: Option<MessageKey>,
    /// Identity of the tracking partial, absent for a single-frame message.
    pub(super) assembly: Option<StreamingAssemblyId>,
    /// Routing metadata captured from the first frame.
    pub(super) parts: RequestParts,
    /// First body chunk.
    pub(super) chunk: Bytes,
    /// Whether this chunk also completes the message.
    pub(super) is_last: bool,
}

/// A continuation chunk offered to the live body.
#[derive(Debug)]
pub(super) struct ChunkOffer {
    /// Message key the chunk belongs to.
    pub(super) key: MessageKey,
    /// Body bytes carried by the frame.
    pub(super) chunk: Bytes,
    /// Whether this chunk completes the message.
    pub(super) is_last: bool,
}

/// Publication of the parked-message count.
///
/// The connection's two parked-count effects go through this pair of function
/// pointers rather than calling the `metrics` statics directly: production
/// wires the process-wide gauge once, and a test can build a state whose
/// effects land in its own tally.
#[derive(Clone, Copy, Debug)]
pub(super) struct ParkedAccounting {
    /// Report one more parked message.
    add: fn(),
    /// Report `count` messages that are no longer parked.
    release: fn(usize),
}

impl ParkedAccounting {
    /// Report through the `metrics` crate's process-wide parked gauge.
    pub(super) const fn process_wide() -> Self {
        Self {
            add: crate::metrics::add_parked_body,
            release: crate::metrics::release_parked_bodies,
        }
    }

    /// Report through a caller's own add and release pair.
    ///
    /// Only a test builds one of these; production always reports through
    /// [`Self::process_wide`].
    #[cfg(test)]
    pub(super) const fn new(add: fn(), release: fn(usize)) -> Self { Self { add, release } }
}

/// Mutable state the connection loop owns across frames.
pub(super) struct LoopState {
    /// The one live streaming exchange, if any.
    pub(super) streams: ConnectionStreams,
    /// Complete messages waiting for the dispatch slot.
    pub(super) parked: ParkedQueue,
    /// Sink the parked count is reported to.
    parked_accounting: ParkedAccounting,
    /// Consecutive malformed-frame count.
    pub(super) deser_failures: u32,
    /// Whether the next wait probes the reader before the push lanes.
    ///
    /// Set after a push batch so a continuously ready push lane cannot win
    /// every biased pass and starve the client's frames.
    pub(super) serve_read_first: bool,
}

impl LoopState {
    /// Create loop state for a fresh connection.
    pub(super) const fn new(queued: QueuedBytes) -> Self {
        Self::with_parked_accounting(queued, ParkedAccounting::process_wide())
    }

    /// Create loop state that reports its parked count through `accounting`.
    ///
    /// The sink is a construction argument rather than a hidden call, so a
    /// test can assert the parked count without touching the shared gauge.
    pub(super) const fn with_parked_accounting(
        queued: QueuedBytes,
        parked_accounting: ParkedAccounting,
    ) -> Self {
        Self {
            streams: ConnectionStreams::new(queued),
            parked: ParkedQueue::new(),
            parked_accounting,
            deser_failures: 0,
            serve_read_first: false,
        }
    }

    /// Park one message and report the new parked count.
    pub(super) fn park(&mut self, message: ParkedMessage) -> io::Result<()> {
        stream_parked::park(&mut self.parked, message)?;
        (self.parked_accounting.add)();
        Ok(())
    }

    /// Take the oldest parked message and report it as no longer parked.
    pub(super) fn pop_parked(&mut self) -> Option<ParkedMessage> {
        let message = self.parked.pop_front();
        if message.is_some() {
            (self.parked_accounting.release)(1);
        }
        message
    }
}

impl Drop for LoopState {
    /// Release the messages this connection still holds when it ends.
    ///
    /// The parked-bodies gauge is a process-wide total, so a connection that
    /// stops with messages still parked must report them. Dropping the state
    /// is the one path every ending takes — normal return, cancellation while
    /// the loop awaits, and unwinding — so the release lives here rather than
    /// at any single exit.
    fn drop(&mut self) { (self.parked_accounting.release)(self.parked.len()); }
}
