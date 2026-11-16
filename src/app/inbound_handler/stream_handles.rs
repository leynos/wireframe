//! Handles and per-chunk data shared by the streaming dispatch modules.
//!
//! The connection loop, the dispatch slot, and the parking queue each need the
//! same four mutable connection handles to answer a streaming handler, plus the
//! read timeout that bounds teardown. Grouping them here keeps every helper
//! within the repository's argument limit, and makes it explicit that a reply
//! travels the same pipeline whichever path produced it.

use std::{collections::VecDeque, time::Duration};

use bytes::Bytes;
use tokio::io::{AsyncRead, AsyncWrite};
use tokio_util::codec::Framed;

use super::{
    super::{
        codec_driver::FramePipeline,
        combined_codec::ConnectionCodec,
        envelope::Packet,
        frame_handling,
    },
    stream_dispatch::DispatchContext,
    stream_parked::{ParkedMessage, parked_bytes},
    stream_slot::ConnectionStreams,
};
use crate::{
    codec::FrameCodec,
    message_assembler::{MessageAssemblyState, MessageKey, StreamingAssemblyId},
    request::RequestParts,
    serializer::Serializer,
};

#[cfg(test)]
#[path = "stream_handles_tests.rs"]
mod tests;

/// Mutable connection handles needed to answer a streaming handler.
///
/// Borrow them for one call with [`DispatchHandles::reborrow`] when the caller
/// must keep using the bundle afterwards.
pub(super) struct DispatchHandles<'a, S, E, F, W>
where
    S: Serializer + Send + Sync,
    E: Packet,
    F: FrameCodec,
    W: AsyncRead + AsyncWrite + Unpin,
{
    /// Connection-local assembly state.
    pub(super) assembly: &'a mut Option<MessageAssemblyState>,
    /// Outbound pipeline shared by every reply on the connection.
    pub(super) pipeline: &'a mut FramePipeline<F::Frame>,
    /// Framed connection the reply is written to.
    pub(super) framed: &'a mut Framed<W, ConnectionCodec<F>>,
    /// Immutable connection inputs.
    pub(super) ctx: &'a DispatchContext<'a, S, E, F>,
    /// Maximum wait for a handler or a body send while teardown runs.
    pub(super) wait: Duration,
}

impl<S, E, F, W> DispatchHandles<'_, S, E, F, W>
where
    S: Serializer + Send + Sync,
    E: Packet,
    F: FrameCodec,
    W: AsyncRead + AsyncWrite + Unpin,
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
    pub(super) const fn reborrow(&mut self) -> DispatchHandles<'_, S, E, F, W> {
        DispatchHandles {
            assembly: &mut *self.assembly,
            pipeline: &mut *self.pipeline,
            framed: &mut *self.framed,
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
    /// including messages parked behind the live body: `send_chunk` admits a
    /// chunk only when the queue, those bytes, and the chunk fit within the
    /// aggregate limit. Omitting the parked queue would let a chunk in and
    /// then trip the hard cap on the next loop step, which aborts the
    /// connection instead of holding back the chunk.
    pub(super) fn measure<S, E, F, W>(
        handles: &DispatchHandles<'_, S, E, F, W>,
        parked: &VecDeque<ParkedMessage>,
    ) -> Self
    where
        S: Serializer + Send + Sync,
        E: Packet,
        F: FrameCodec,
        W: AsyncRead + AsyncWrite + Unpin,
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
    parked: &VecDeque<ParkedMessage>,
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
/// Mutable state the connection loop owns across frames.
pub(super) struct LoopState {
    /// The one live streaming exchange, if any.
    pub(super) streams: ConnectionStreams,
    /// Complete messages waiting for the dispatch slot.
    pub(super) parked: VecDeque<ParkedMessage>,
    /// Consecutive malformed-frame count.
    pub(super) deser_failures: u32,
}

impl LoopState {
    /// Create loop state for a fresh connection.
    pub(super) fn new() -> Self {
        Self {
            streams: ConnectionStreams::new(),
            parked: VecDeque::new(),
            deser_failures: 0,
        }
    }
}
