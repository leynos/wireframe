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

use super::{stream_diagnostics, stream_slot::HandlerFuture};
use crate::request::AccountedBodySender;

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
    /// The wait elapsed with the handler still running.
    Deadline,
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
    error: io::Error,
    sender: AccountedBodySender,
    handler: &mut HandlerFuture,
) -> HandlerWait {
    let delivery = deliver_terminal(wait, error, sender);
    tokio::pin!(delivery);
    let deadline = tokio::time::sleep(wait);
    tokio::pin!(deadline);
    let mut is_delivered = false;
    loop {
        tokio::select! {
            () = &mut delivery, if !is_delivered => is_delivered = true,
            outcome = handler.as_mut() => return HandlerWait::Replied(outcome),
            () = &mut deadline => return HandlerWait::Deadline,
        }
    }
}

/// Tell the handler that its body failed, without waiting indefinitely.
async fn deliver_terminal(wait: Duration, error: io::Error, sender: AccountedBodySender) {
    let delivered = timeout(wait, sender.send_terminal(error)).await;
    let Ok(outcome) = delivered else {
        stream_diagnostics::terminal_delivery_timed_out();
        return;
    };
    if let Err(send_error) = outcome {
        stream_diagnostics::terminal_receiver_gone(send_error);
    }
}
