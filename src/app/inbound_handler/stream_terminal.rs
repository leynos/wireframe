//! Terminating a streaming request body while its handler is still running.
//!
//! A body failure needs a slot on the same bounded channel that carries body
//! chunks, and only the handler can free one. These helpers therefore deliver
//! the failure while polling the handler, rather than before or after it, and
//! bound the whole exchange so an unresponsive handler cannot hold the
//! connection open.

use std::{future::Future, io, time::Duration};

use bytes::Bytes;
use tokio::time::timeout;

use super::{
    stream_active::{BodyIdentity, HandlerFuture, PendingSend},
    stream_diagnostics,
};
use crate::{metrics::StreamingBodyOutcome, request::AccountedBodySender};

#[cfg(test)]
#[cfg(feature = "metrics")]
#[path = "stream_outcome_delivery_tests.rs"]
mod outcome_tests;

/// How a handler finished while the connection was waiting for its reply.
///
/// A handler that returns `None` has deliberately chosen not to reply, which is
/// a normal outcome. A handler still running when the wait expires has produced
/// no answer at all, and the connection has abandoned it. Keeping the two apart
/// means the abandoned case can be reported instead of silently looking like a
/// deliberate silence.
#[derive(Debug, Eq, PartialEq)]
pub(super) enum HandlerWait {
    /// The handler finished within the wait, with or without a reply.
    Replied(Option<Bytes>),
    /// The handler produced no answer the connection could wait for.
    Deadline,
}

/// One body failure the handler has yet to observe.
pub(super) struct TerminalFailure {
    /// Message whose body failed.
    pub(super) identity: BodyIdentity,
    /// Failure the handler should observe instead of a clean end of stream.
    pub(super) error: io::Error,
}

/// What became of an attempt to deliver a body failure to its handler.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum Delivery {
    /// The handler's channel accepted the failure.
    Accepted,
    /// The channel did not accept it within the wait.
    TimedOut,
    /// The handler had already dropped its body.
    ReceiverGone,
}

/// Await the handler under the connection's teardown bound.
pub(super) async fn await_handler(wait: Duration, handler: &mut HandlerFuture) -> HandlerWait {
    timeout(wait, handler.as_mut())
        .await
        .map_or_else(|_elapsed| HandlerWait::Deadline, HandlerWait::Replied)
}

/// Deliver a terminal body error while the handler drains, then await it.
///
/// The error needs a slot on the same bounded channel that carries body chunks,
/// so delivering it before polling the handler would deadlock whenever that
/// channel is full: only the handler can free a slot, and it is not running.
/// The timeout on the delivery would then drop the error, the sender would
/// close, and a truncated body would look like a clean end of stream.
///
/// Polling both together lets the handler take the queued chunks and then
/// observe the failure. The wait bounds the whole exchange, so a handler that
/// never reads still cannot hold the connection open.
pub(super) async fn deliver_then_await(
    wait: Duration,
    failure: TerminalFailure,
    sender: AccountedBodySender,
    handler: &mut HandlerFuture,
) -> HandlerWait {
    race_delivery(wait, deliver_terminal(wait, failure, sender), handler).await
}

/// Deliver a body's completed final chunk, then await the handler.
///
/// The body is already whole when this runs: the chunk that ends it had been
/// read from the socket before an inbound failure closed the exchange, so
/// nothing was truncated. Handing that chunk over therefore needs no terminal
/// error, and reporting one instead would turn a complete upload into a
/// truncated body.
///
/// The delivery is unmetered. Admission is what stalled before the close, and
/// these bytes have no other destination, so the aggregate limit and the queue
/// cap would only lose a body the handler is owed. The chunk still takes its
/// place in the channel behind the chunks already queued, so the handler sees
/// it in order.
pub(super) async fn deliver_final_chunk_then_await(
    wait: Duration,
    identity: BodyIdentity,
    pending: PendingSend,
    handler: &mut HandlerFuture,
) -> HandlerWait {
    let sending = send_final_chunk(wait, identity, pending);
    race_delivery(wait, sending, handler).await
}

/// Race one delivery against the handler and the connection's wait.
///
/// Racing rather than sequencing is what lets a delivery that needs a channel
/// slot make progress: only the handler can free one, and it is not running
/// while a delivery is awaited on its own.
async fn race_delivery<D>(wait: Duration, delivery: D, handler: &mut HandlerFuture) -> HandlerWait
where
    D: Future<Output = Delivery>,
{
    tokio::pin!(delivery);
    let deadline = tokio::time::sleep(wait);
    tokio::pin!(deadline);
    let mut delivered: Option<Delivery> = None;
    loop {
        tokio::select! {
            // Biased so a settled delivery is acted on before the handler is
            // polled again: when the failure could not be handed over, its
            // reply cannot describe that failure.
            biased;
            outcome = &mut delivery, if delivered.is_none() => {
                delivered = Some(outcome);
                if outcome == Delivery::TimedOut {
                    return HandlerWait::Deadline;
                }
            }
            outcome = handler.as_mut() => {
                // The reply settled before the failure did, so the body was
                // abandoned undelivered. `deliver_terminal` records the
                // outcome for every delivery it completes itself, and this
                // branch is only reachable while none has.
                if delivered.is_none() {
                    StreamingBodyOutcome::Abandoned.record();
                }
                return HandlerWait::Replied(outcome);
            }
            () = &mut deadline => return HandlerWait::Deadline,
        }
    }
}

/// Hand a completed final chunk to its handler, reporting what became of it.
///
/// A final chunk is the last thing a body sends, so no later chunk can report
/// its fate: the outcome recorded here is the whole record of the body.
async fn send_final_chunk(
    wait: Duration,
    identity: BodyIdentity,
    pending: PendingSend,
) -> Delivery {
    let PendingSend { sender, chunk, .. } = pending;
    match timeout(wait, sender.send_chunk(chunk, 0, usize::MAX)).await {
        Ok(Ok(())) => {
            StreamingBodyOutcome::Delivered.record();
            Delivery::Accepted
        }
        Ok(Err(gone)) => {
            stream_diagnostics::final_chunk_refused(identity, gone);
            StreamingBodyOutcome::Abandoned.record();
            Delivery::ReceiverGone
        }
        // The handler never made room, so the body it was owed is abandoned.
        // The wait already returned `Deadline`, which suppresses the reply.
        Err(_elapsed) => {
            StreamingBodyOutcome::Abandoned.record();
            Delivery::TimedOut
        }
    }
}

/// Tell the handler that its body failed, without waiting indefinitely.
///
/// The outcome is returned rather than only logged, because it decides whether
/// waiting for the handler can still produce a reply worth forwarding.
async fn deliver_terminal(
    wait: Duration,
    failure: TerminalFailure,
    sender: AccountedBodySender,
) -> Delivery {
    let TerminalFailure { identity, error } = failure;
    match timeout(wait, sender.send_terminal(error)).await {
        Ok(Ok(())) => {
            StreamingBodyOutcome::Terminal.record();
            Delivery::Accepted
        }
        Ok(Err(gone)) => {
            stream_diagnostics::terminal_receiver_gone(identity, gone);
            StreamingBodyOutcome::Abandoned.record();
            Delivery::ReceiverGone
        }
        Err(_elapsed) => {
            stream_diagnostics::terminal_delivery_timed_out(identity);
            StreamingBodyOutcome::DeliveryTimeout.record();
            Delivery::TimedOut
        }
    }
}

/// What closing a live exchange must deliver before awaiting its handler.
///
/// A body that completes while it closes is not a failure: a whole message
/// followed by a half-close is a common client shape, and reporting it as a
/// truncated body would turn a complete upload into an error reply.
pub(super) enum BodyClose {
    /// The body completed, and its final chunk still has to reach the handler.
    FinalChunk(PendingSend),
    /// The body was truncated, and the handler is owed the reason.
    Fail {
        /// Failure the handler should observe.
        failure: TerminalFailure,
        /// Open body sender the failure is delivered through.
        sender: AccountedBodySender,
    },
    /// Nothing can be delivered, so only the handler's own reply remains.
    AwaitReply,
}

/// Decide what a closing exchange still owes its handler.
///
/// A pending final chunk takes precedence: it ends the body, so the failure
/// that closed the exchange has nothing left to truncate.
pub(super) fn body_close(
    pending: Option<PendingSend>,
    terminal: Option<io::Error>,
    sender: Option<AccountedBodySender>,
    identity: BodyIdentity,
) -> BodyClose {
    match (pending, terminal, sender) {
        (Some(final_chunk), ..) if final_chunk.is_last() => BodyClose::FinalChunk(final_chunk),
        (_, Some(error), Some(open_sender)) => BodyClose::Fail {
            failure: TerminalFailure { identity, error },
            sender: open_sender,
        },
        _ => BodyClose::AwaitReply,
    }
}
