//! Terminating a streaming request body while its handler is still running.
//!
//! A body failure needs a slot on the same bounded channel that carries body
//! chunks, and only the handler can free one. These helpers therefore deliver
//! the failure while polling the handler, rather than before or after it, and
//! bound the whole exchange so an unresponsive handler cannot hold the
//! connection open.

use std::{io, time::Duration};

use bytes::Bytes;
use tokio::time::timeout;

use super::{
    stream_active::{BodyIdentity, HandlerFuture},
    stream_diagnostics,
};
use crate::{metrics::StreamingBodyOutcome, request::AccountedBodySender};

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
    let sending = deliver_terminal(wait, failure, sender);
    tokio::pin!(sending);
    let deadline = tokio::time::sleep(wait);
    tokio::pin!(deadline);
    let mut delivered: Option<Delivery> = None;
    loop {
        tokio::select! {
            // Biased so a settled delivery is acted on before the handler is
            // polled again: when the failure could not be handed over, its
            // reply cannot describe that failure.
            biased;
            outcome = &mut sending, if delivered.is_none() => {
                delivered = Some(outcome);
                if outcome == Delivery::TimedOut {
                    return HandlerWait::Deadline;
                }
            }
            outcome = handler.as_mut() => return HandlerWait::Replied(outcome),
            () = &mut deadline => return HandlerWait::Deadline,
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
