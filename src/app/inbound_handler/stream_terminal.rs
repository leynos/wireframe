//! Terminating a streaming request body while its handler is still running.
//!
//! A body failure needs a slot on the same bounded channel that carries body
//! chunks, and only the handler can free one. These helpers therefore deliver
//! the failure while polling the handler, rather than before or after it, and
//! bound the whole exchange so an unresponsive handler cannot hold the
//! connection open.

use std::{io, time::Duration};

use log::debug;
use tokio::time::timeout;

use super::stream_slot::HandlerFuture;
use crate::request::AccountedBodySender;

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
) -> Option<Vec<u8>> {
    let delivery = deliver_terminal(wait, error, sender);
    tokio::pin!(delivery);
    let deadline = tokio::time::sleep(wait);
    tokio::pin!(deadline);
    let mut is_delivered = false;
    loop {
        tokio::select! {
            () = &mut delivery, if !is_delivered => is_delivered = true,
            outcome = handler.as_mut() => return outcome,
            () = &mut deadline => return None,
        }
    }
}

/// Tell the handler that its body failed, without waiting indefinitely.
async fn deliver_terminal(wait: Duration, error: io::Error, sender: AccountedBodySender) {
    match timeout(wait, sender.send_terminal(error)).await {
        Ok(Ok(())) => {}
        Ok(Err(send_error)) => {
            debug!("request body receiver gone before the failure was delivered: {send_error:?}");
        }
        Err(_) => debug!("timed out delivering the request body failure"),
    }
}
