//! The live streaming exchange on one connection.
//!
//! A connection admits one request body at a time. The exchange that owns it
//! keeps the handler future, the body sender, and at most one in-flight chunk
//! admission together, so the loop never has to reconstruct a partially
//! consumed exchange, and dropping it cancels the handler and releases queued
//! bytes in one step.
//!
//! Only the exchange lives here. The slot that owns it, and the account every
//! chunk is charged to, are in [`stream_slot`](super::stream_slot).

use std::{future::Future, pin::Pin, time::Duration};

use bytes::Bytes;
use tokio::time::timeout;

use super::stream_handles::ChunkBudget;
use crate::{
    message_assembler::{MessageKey, StreamingAssemblyId},
    metrics::StreamingBodyOutcome,
    request::{AccountedBodySender, SendChunkError},
};

/// Future returned by a streaming route handler.
pub(super) type HandlerFuture = Pin<Box<dyn Future<Output = Option<Bytes>> + Send + 'static>>;

/// One step while a chunk admission is in flight.
pub(super) enum PendingEvent {
    /// The chunk was queued, or the consumer had gone away.
    Send(Result<(), SendChunkError>),
    /// The handler resolved first; the admission is cancelled.
    Handler(Option<Bytes>),
    /// The bounded wait elapsed with the admission still queued.
    Idle,
}

/// Identity of the request whose body is streaming.
///
/// Carried into the terminal paths so a diagnostic can name the message
/// without the helper reaching back into the exchange it is tearing down.
#[derive(Clone, Copy, Debug)]
pub(super) struct BodyIdentity {
    /// Route identifier of the message.
    pub(super) message_id: u32,
    /// Correlation identifier inherited from the first frame, when present.
    pub(super) correlation_id: Option<u64>,
}

/// The one live streaming exchange on a connection.
pub(super) struct ActiveStream {
    /// Assembly key used to match continuation frames.
    pub(super) key: Option<MessageKey>,
    /// Identity of the partial this exchange owns, absent for a single-frame
    /// message or when no assembler participates.
    pub(super) assembly: Option<StreamingAssemblyId>,
    /// Route identifier used to address the reply.
    pub(super) route_id: u32,
    /// Correlation identifier inherited from the first frame.
    pub(super) correlation_id: Option<u64>,
    /// Body sender; `None` once the body is complete or abandoned.
    pub(super) sender: Option<AccountedBodySender>,
    /// Handler awaiting the body.
    pub(super) handler: HandlerFuture,
    /// Chunk admission in flight.
    pub(super) pending: Option<PendingSend>,
}

/// One chunk waiting for byte admission and channel capacity.
///
/// The chunk is kept rather than a future that admitted it, because admission
/// is re-decided on every drive: the bytes retained outside the queue can only
/// shrink while a chunk waits, and a chunk refused against a stale, larger
/// total would be affordable moments later. Re-reserving is safe, because a
/// cancelled attempt releases whatever it had reserved.
pub(super) struct PendingSend {
    /// Sender the chunk is admitted to.
    pub(super) sender: AccountedBodySender,
    /// The chunk still waiting for admission.
    pub(super) chunk: Bytes,
    /// Whether this chunk also completes the message.
    pub(super) is_last: bool,
}

impl ActiveStream {
    /// Return the identity of the message this exchange is serving.
    pub(super) const fn identity(&self) -> BodyIdentity {
        BodyIdentity {
            message_id: self.route_id,
            correlation_id: self.correlation_id,
        }
    }

    /// Drive the in-flight chunk admission alongside the handler.
    ///
    /// The wait is bounded by `wait` so the caller regains control even when
    /// neither side makes progress. An inbound frame that arrives while the
    /// chunk is queued is therefore not dispatched from here: a continuation
    /// for the same message would be mistaken for an abandoned body. The
    /// caller keeps such a frame for the step after admission, and the timeout
    /// is what keeps assembly expiry and the memory budget observable while
    /// the chunk waits.
    pub(super) async fn drive_pending(
        &mut self,
        wait: Duration,
        budget: ChunkBudget,
    ) -> PendingEvent {
        let Self {
            pending, handler, ..
        } = self;
        let Some(send) = pending else {
            // The caller only drives this branch when a send is pending.
            return PendingEvent::Send(Ok(()));
        };
        // Rebuilt from the current budget on every drive. The measurement taken
        // when the chunk was queued can only overstate what is retained
        // elsewhere, and overstating it refuses a chunk that has since become
        // affordable; the queued chunk itself is preserved across attempts.
        let admission =
            send.sender
                .clone()
                .send_chunk(send.chunk.clone(), budget.other_retained, budget.limit);
        let exchange = async {
            tokio::select! {
                result = admission => PendingEvent::Send(result),
                outcome = handler.as_mut() => PendingEvent::Handler(outcome),
            }
        };
        match timeout(wait, exchange).await {
            Ok(event) => event,
            Err(_elapsed) => {
                // The chunk is still unadmitted, so the connection reports a
                // body that cannot make progress rather than one that ended.
                StreamingBodyOutcome::AdmissionTimeout.record();
                PendingEvent::Idle
            }
        }
    }

    /// Poll the handler until it produces a reply.
    ///
    /// The connection loop selects this future against the next inbound read,
    /// so a handler that finishes while its body is still arriving is not
    /// delayed behind a peer that sends nothing.
    pub(super) async fn poll_handler(&mut self) -> Option<Bytes> { (&mut self.handler).await }
}
