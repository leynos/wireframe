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
//! holds it, its admission bookkeeping — including the stall clock that bounds
//! a chunk the handler never takes — and the paths that start and advance it.
//! The terminal paths that free the slot are in
//! [`stream_close`](super::stream_close).

use std::{io, time::Duration};

use tokio::time::Instant;

use super::{
    super::StreamingHandler,
    stream_active::{ActiveStream, PendingEvent, PendingSend},
    stream_diagnostics,
    stream_handles::{ChunkBudget, ChunkOffer, FirstChunk},
};
#[cfg(test)]
#[path = "stream_slot_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "stream_admission_tests.rs"]
mod admission_tests;

#[cfg(test)]
#[cfg(feature = "metrics")]
#[path = "stream_outcome_slot_tests.rs"]
mod outcome_tests;

use crate::{
    message_assembler::{MessageKey, StreamingAssemblyId},
    metrics::StreamingBodyOutcome,
    request::{DEFAULT_BODY_CHANNEL_CAPACITY, QueuedBytes, SendChunkError, accounted_body_channel},
};

/// A body the frame layer ended with an error.
///
/// The key names the message whose body failed, and the error is what the
/// handler is owed in place of the rest of its body. Grouped rather than passed
/// separately so the failure travels through the close path as one value.
pub(super) struct BodyFailure {
    /// Message whose body failed.
    pub(super) key: MessageKey,
    /// Error the handler should observe instead of a clean end of stream.
    pub(super) error: io::Error,
}

/// Connection-owned streaming state: the dispatch slot and byte account.
pub(super) struct ConnectionStreams {
    /// Bytes queued for the live body and released as the consumer reads.
    queued: QueuedBytes,
    /// The single live streaming exchange, if any.
    pub(super) active: Option<ActiveStream>,
    /// When the live body's chunk admission first failed to make progress.
    ///
    /// A chunk that cannot be admitted is retried on every step for as long as
    /// its handler declines to read, so without a record of when the wait
    /// started nothing in the loop can bound it. The stamp survives the
    /// individual waits and is cleared as soon as an admission settles, so it
    /// measures one stall rather than the body's whole life.
    stalled_since: Option<Instant>,
}

impl ConnectionStreams {
    /// Create an empty slot sharing the connection's byte account.
    pub(super) const fn new(queued: QueuedBytes) -> Self {
        Self {
            queued,
            active: None,
            stalled_since: None,
        }
    }

    /// Clone the connection-wide byte account.
    ///
    /// A pre-filled body is charged to the same account as an incrementally
    /// delivered one, so both forms observe one aggregate budget.
    pub(super) fn queued(&self) -> QueuedBytes { self.queued.clone() }

    /// Return whether a streaming body currently owns the slot.
    pub(super) const fn is_live(&self) -> bool { self.active.is_some() }

    /// Return whether the live body's admission has stalled past `bound`.
    ///
    /// A body whose chunk cannot clear admission has no other exit when its
    /// handler never reads it: the reader is held so the peer cannot push past
    /// the byte budget, and no frame can be dispatched until the chunk
    /// settles. The bound is what keeps such a connection from being pinned
    /// for the life of the server.
    pub(super) fn stall_expired(&self, bound: Duration) -> bool {
        self.stalled_since
            .is_some_and(|since| since.elapsed() >= bound)
    }

    /// Return how much of the stall bound one more drive may consume.
    ///
    /// A drive that is already stalled must not run past the point where the
    /// body should be failed, so the caller caps its wait by what remains. A
    /// body with no stall yet is offered a whole interval, because the drive
    /// that starts is the one that begins measuring it.
    pub(super) fn stall_remaining(&self, bound: Duration) -> Duration {
        self.stalled_since
            .map_or(bound, |since| bound.saturating_sub(since.elapsed()))
    }

    /// Record whether the last admission drive made progress.
    ///
    /// Only a drive that timed out with the chunk still queued counts as a
    /// stall: a settled send, a refused channel, and a handler that resolved
    /// first all end the wait, so the next stall starts its own interval.
    ///
    /// The interval starts when the drive began waiting, not when it gave up,
    /// because a wait that expires is itself the time the chunk went
    /// unadmitted. Recording the later instant would restart the bound on
    /// every drive and no stall would ever reach it.
    fn note_admission(&mut self, event: &PendingEvent, started: Instant) {
        if matches!(event, PendingEvent::Idle) {
            self.stalled_since.get_or_insert(started);
        } else {
            self.stalled_since = None;
        }
    }

    /// Return the bytes queued for the consumer.
    pub(super) fn retained_bytes(&self) -> usize { self.queued.queued() }

    /// Forget the stall interval recorded for the live body.
    ///
    /// Called when an exchange ends: the body that accumulated the stall is
    /// gone, so the next one must start its own interval rather than inherit
    /// a wait it never performed.
    pub(super) const fn forget_stall(&mut self) { self.stalled_since = None; }

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
    ///
    /// Each drive is bounded so the step can re-check shutdown, expiry and the
    /// memory budget, and the stall it reports is accumulated across drives:
    /// bounding one drive alone would let a body wait forever, while bounding
    /// the drive for the whole stall would withhold those checks for as long
    /// as the stall lasts.
    pub(super) async fn drive_pending(
        &mut self,
        wait: Duration,
        budget: ChunkBudget,
    ) -> PendingEvent {
        let Some(active) = self.active.as_mut() else {
            return PendingEvent::Send(Ok(()));
        };
        let started = Instant::now();
        let event = active.drive_pending(wait, budget).await;
        self.note_admission(&event, started);
        event
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
        self.stalled_since = None;
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

    /// Drop the live exchange immediately after a hard budget breach.
    pub(super) fn abort(&mut self) {
        self.forget_stall();
        if self.active.take().is_some() {
            StreamingBodyOutcome::Abandoned.record();
        }
    }

    /// Return whether the slot owns one of the evicted assemblies.
    pub(super) fn owns_assembly(&self, ids: &[StreamingAssemblyId]) -> bool {
        self.active
            .as_ref()
            .and_then(|active| active.assembly)
            .is_some_and(|id| ids.contains(&id))
    }

    /// Return whether the slot is live for one of `keys`.
    pub(super) fn is_key_live(&self, keys: &[MessageKey]) -> bool {
        self.active
            .as_ref()
            .and_then(|active| active.key)
            .is_some_and(|key| keys.contains(&key))
    }
}
