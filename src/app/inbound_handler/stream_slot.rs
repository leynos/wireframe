//! Connection-owned streaming slot.
//!
//! The slot holds everything that outlives a single inbound frame while a
//! request body streams: the live exchange, and the byte account every chunk
//! of that body is charged to. Keeping them together means the loop never has
//! to reconstruct a partially consumed exchange, and dropping the slot cancels
//! the handler and releases queued bytes in one step.
//!
//! The exchange itself, with its pending chunk admission, is in
//! [`stream_active`](super::stream_active); this module owns the slot that
//! holds it and the terminal paths that free it.

use std::{io, time::Duration};

use bytes::Bytes;
use futures::Sink;

use super::{
    super::{
        StreamingHandler,
        envelope::{Envelope, Packet},
        frame_handling::Dispatch,
    },
    stream_active::{ActiveStream, PendingEvent, PendingSend},
    stream_diagnostics,
    stream_handles::{ChunkBudget, ChunkOffer, DispatchHandles, FirstChunk},
    stream_parked::forward_reply,
    stream_terminal::{HandlerWait, TerminalFailure, await_handler, deliver_then_await},
};
#[cfg(test)]
#[path = "stream_slot_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "stream_admission_tests.rs"]
mod admission_tests;

#[cfg(all(test, feature = "metrics"))]
#[path = "stream_outcome_tests.rs"]
mod outcome_tests;

use crate::{
    codec::FrameCodec,
    message::EncodeWith,
    message_assembler::{MessageAssemblyState, MessageKey, StreamingAssemblyId},
    metrics::StreamingBodyOutcome,
    request::{DEFAULT_BODY_CHANNEL_CAPACITY, QueuedBytes, SendChunkError, accounted_body_channel},
    serializer::Serializer,
};

/// Connection-owned streaming state: the dispatch slot and byte account.
pub(super) struct ConnectionStreams {
    /// Bytes queued for the live body and released as the consumer reads.
    queued: QueuedBytes,
    /// The single live streaming exchange, if any.
    pub(super) active: Option<ActiveStream>,
}

impl ConnectionStreams {
    /// Create an empty slot sharing the connection's byte account.
    pub(super) const fn new(queued: QueuedBytes) -> Self {
        Self {
            queued,
            active: None,
        }
    }

    /// Clone the connection-wide byte account.
    ///
    /// A pre-filled body is charged to the same account as an incrementally
    /// delivered one, so both forms observe one aggregate budget.
    pub(super) fn queued(&self) -> QueuedBytes { self.queued.clone() }

    /// Return whether a streaming body currently owns the slot.
    pub(super) const fn is_live(&self) -> bool { self.active.is_some() }

    /// Return the bytes queued for the consumer.
    pub(super) fn retained_bytes(&self) -> usize { self.queued.queued() }

    /// Return the bytes of the chunk still waiting for admission, if any.
    ///
    /// The chunk is not charged to the account until it is admitted, so a
    /// reader that wants the connection's whole retained total has to add it
    /// to [`Self::retained_bytes`].
    #[cfg(feature = "test-support")]
    pub(super) fn pending_chunk_len(&self) -> usize {
        self.active
            .as_ref()
            .and_then(|active| active.pending.as_ref())
            .map_or(0, |pending| pending.chunk.len())
    }

    /// Return whether a chunk admission is in flight.
    pub(super) fn has_pending_send(&self) -> bool {
        self.active
            .as_ref()
            .is_some_and(|active| active.pending.is_some())
    }

    /// Drive the in-flight chunk admission on the live exchange.
    pub(super) async fn drive_pending(
        &mut self,
        wait: Duration,
        budget: ChunkBudget,
    ) -> PendingEvent {
        match self.active.as_mut() {
            Some(active) => active.drive_pending(wait, budget).await,
            None => PendingEvent::Send(Ok(())),
        }
    }

    /// Start a streaming exchange and queue its first chunk.
    ///
    /// The first chunk becomes a pending admission rather than being awaited
    /// here. Admission is decided against the connection-wide byte account, so
    /// awaiting it would stall the loop on bytes this step cannot release:
    /// messages parked behind the body are only dispatched once the slot frees,
    /// and bytes an earlier body still holds are only released by a handler
    /// this call has not polled. Registering the slot first lets the loop drive
    /// the admission alongside the handler and the inbound socket, exactly as
    /// it drives every continuation chunk.
    ///
    /// A closed consumer simply leaves the handler to observe an empty body.
    pub(super) fn start(&mut self, first: FirstChunk, handler: &StreamingHandler) {
        let FirstChunk {
            key,
            assembly,
            parts,
            chunk,
            is_last,
        } = first;
        let route_id = parts.id();
        let correlation_id = parts.correlation_id();
        let (sender, body) =
            accounted_body_channel(DEFAULT_BODY_CHANNEL_CAPACITY, self.queued.clone());
        let handler_future = handler(parts, body);
        // The final chunk still has to reach the handler, so the admission owns
        // its own sender clone until the loop records the result.
        let live_sender = (!is_last).then(|| sender.clone());
        self.active = Some(ActiveStream {
            key,
            assembly,
            route_id,
            correlation_id,
            sender: live_sender,
            handler: handler_future,
            pending: Some(PendingSend {
                sender,
                chunk,
                is_last,
            }),
        });
    }

    /// Queue one continuation chunk without reading further frames.
    ///
    /// The chunk is stored unadmitted: it is admitted against a fresh budget
    /// each time the loop drives it, so bytes released while it waits are
    /// observed.
    pub(super) fn push_chunk(&mut self, offer: ChunkOffer) -> bool {
        let Some(active) = self.active.as_mut() else {
            return false;
        };
        if active.key != Some(offer.key) || active.pending.is_some() {
            return false;
        }
        let Some(sender) = active.sender.clone() else {
            return false;
        };
        let ChunkOffer { chunk, is_last, .. } = offer;
        active.pending = Some(PendingSend {
            sender,
            chunk,
            is_last,
        });
        true
    }

    /// Record an admission result and close the body after the final chunk.
    pub(super) fn complete_send(&mut self, result: Result<(), SendChunkError>) {
        let Some(active) = self.active.as_mut() else {
            return;
        };
        let Some(pending) = active.pending.take() else {
            return;
        };
        if pending.is_last {
            // No later chunk can report the final one's fate, because a final
            // chunk is the last thing the body sends. A handler that had
            // already dropped its stream therefore leaves the body abandoned
            // rather than uncounted, which matches the parked path.
            match result {
                // The final chunk reached the handler, so the body is
                // delivered whatever the handler does with it afterwards.
                Ok(()) => StreamingBodyOutcome::Delivered.record(),
                Err(error) => {
                    stream_diagnostics::final_chunk_refused(active.identity(), error);
                    StreamingBodyOutcome::Abandoned.record();
                }
            }
            active.sender = None;
        } else if result.is_err() {
            // A refused earlier chunk is reported per chunk until the body
            // ends, so counting it here would double-count.
            active.sender = None;
        }
    }

    /// Finish the exchange after the handler resolved on its own.
    pub(super) async fn finish_active<S, E, K, F>(
        &mut self,
        outcome: Option<Bytes>,
        handles: DispatchHandles<'_, S, E, K, F>,
    ) -> io::Result<Dispatch>
    where
        S: Serializer + Send + Sync,
        E: Packet,
        F: FrameCodec,
        K: Sink<F::Frame, Error = io::Error> + Unpin + Send,
        Envelope: EncodeWith<S>,
    {
        let Some(mut active) = self.active.take() else {
            return Ok(Dispatch::Continue);
        };
        // Cancelling the admission is safe: a reserved permit that never sent
        // releases itself, and a queued chunk is dropped with its guard.
        if active.pending.take().is_some() {
            // The handler replied before the chunk it was waiting for arrived,
            // so that chunk is abandoned with the exchange.
            StreamingBodyOutcome::Abandoned.record();
        }
        active.sender = None;
        discard_streaming(active.assembly, handles.assembly);
        forward_reply(active.route_id, active.correlation_id, outcome, handles).await
    }

    /// Fail the live body, await its handler, and forward any reply.
    pub(super) async fn close_active<S, E, K, F>(
        &mut self,
        terminal: Option<io::Error>,
        handles: DispatchHandles<'_, S, E, K, F>,
    ) -> io::Result<Dispatch>
    where
        S: Serializer + Send + Sync,
        E: Packet,
        F: FrameCodec,
        K: Sink<F::Frame, Error = io::Error> + Unpin + Send,
        Envelope: EncodeWith<S>,
    {
        let Some(mut active) = self.active.take() else {
            return Ok(Dispatch::Continue);
        };
        active.pending = None;
        let sender = active.sender.take();
        discard_streaming(active.assembly, handles.assembly);
        let wait = match (terminal, sender) {
            (Some(error), Some(open_sender)) => {
                let failure = TerminalFailure {
                    identity: active.identity(),
                    error,
                };
                deliver_then_await(handles.wait, failure, open_sender, &mut active.handler).await
            }
            _ => await_handler(handles.wait, &mut active.handler).await,
        };
        let reply = match wait {
            HandlerWait::Replied(reply) => reply,
            HandlerWait::Deadline => {
                // The handler outlived the wait and its reply is abandoned.
                // Recording it keeps a hung handler distinguishable from one
                // that deliberately chose not to reply.
                stream_diagnostics::handler_deadline_exceeded(active.route_id);
                None
            }
        };
        forward_reply(active.route_id, active.correlation_id, reply, handles).await
    }

    /// Fail the body for an expired streaming assembly.
    ///
    /// The evicted assemblies are matched by identity, not by key: a
    /// replacement may already have reused the key, and it must keep its
    /// own body.
    pub(super) async fn expire<S, E, K, F>(
        &mut self,
        ids: &[StreamingAssemblyId],
        handles: DispatchHandles<'_, S, E, K, F>,
    ) -> io::Result<Dispatch>
    where
        S: Serializer + Send + Sync,
        E: Packet,
        F: FrameCodec,
        K: Sink<F::Frame, Error = io::Error> + Unpin + Send,
        Envelope: EncodeWith<S>,
    {
        if !self.owns_assembly(ids) {
            return Ok(Dispatch::Continue);
        }
        self.close_active(
            Some(io::Error::new(
                io::ErrorKind::TimedOut,
                "request body assembly expired before completion",
            )),
            handles,
        )
        .await
    }

    /// Fail the body for a terminal assembly rejection.
    pub(super) async fn fail<S, E, K, F>(
        &mut self,
        key: MessageKey,
        error: io::Error,
        handles: DispatchHandles<'_, S, E, K, F>,
    ) -> io::Result<Dispatch>
    where
        S: Serializer + Send + Sync,
        E: Packet,
        F: FrameCodec,
        K: Sink<F::Frame, Error = io::Error> + Unpin + Send,
        Envelope: EncodeWith<S>,
    {
        if !self.is_key_live(std::slice::from_ref(&key)) {
            stream_diagnostics::no_live_handler(key);
            return Ok(Dispatch::Continue);
        }
        self.close_active(Some(error), handles).await
    }

    /// Drop the live exchange immediately after a hard budget breach.
    pub(super) fn abort(&mut self) {
        if self.active.take().is_some() {
            StreamingBodyOutcome::Abandoned.record();
        }
    }

    /// Return whether the slot owns one of the evicted assemblies.
    fn owns_assembly(&self, ids: &[StreamingAssemblyId]) -> bool {
        self.active
            .as_ref()
            .and_then(|active| active.assembly)
            .is_some_and(|id| ids.contains(&id))
    }

    /// Return whether the slot is live for one of `keys`.
    fn is_key_live(&self, keys: &[MessageKey]) -> bool {
        self.active
            .as_ref()
            .and_then(|active| active.key)
            .is_some_and(|key| keys.contains(&key))
    }
}

/// Switch one streaming assembly to discard mode if state is present.
///
/// The exchange names the assembly it owns, so an assembly that reused the key
/// after this one expired keeps receiving its frames.
fn discard_streaming(id: Option<StreamingAssemblyId>, assembly: &mut Option<MessageAssemblyState>) {
    if let (Some(owned), Some(assembly_state)) = (id, assembly.as_mut()) {
        assembly_state.discard_streaming_if(owned);
    }
}
