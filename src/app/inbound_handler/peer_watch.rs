//! Peer-disconnect observation for in-flight handler responses.
//!
//! A handler may stream for an unbounded time. Waiting for it before reading
//! the next inbound frame would hide a peer that has already gone away, leaving
//! the connection task, its gauge guard, and the server drain pending forever.
//! These helpers poll the read half while one frame is being handled.
//!
//! Cancellation policy:
//!
//! - A read error cancels the in-flight response and fails the connection.
//! - A clean EOF means the peer half-closed its write side. It may still be reading, so the
//!   response is allowed to finish; the shared token lets the actor bound how long it waits for a
//!   producer that never yields again.

use std::io;

use futures::{Stream, StreamExt};
use tokio::time::{Duration, timeout};
use tokio_util::sync::CancellationToken;

/// Outcome of waiting for the next inbound frame.
pub(super) enum NextFrame<F> {
    /// A frame is ready for dispatch.
    Ready(F),
    /// The peer closed its input half without sending another frame.
    Closed,
    /// No frame arrived within the configured read timeout.
    TimedOut,
}

/// Wait for the next inbound frame, reusing one buffered during dispatch.
///
/// A frame read while the previous one was being handled is returned first, so
/// the read half is never polled twice for one inbound frame.
pub(super) async fn next_inbound_frame<R, F>(
    pending_frame: &mut Option<F>,
    reader: &mut R,
    read_timeout_ms: u64,
) -> io::Result<NextFrame<F>>
where
    R: Stream<Item = io::Result<F>> + Unpin,
{
    if let Some(frame) = pending_frame.take() {
        return Ok(NextFrame::Ready(frame));
    }
    match timeout(Duration::from_millis(read_timeout_ms), reader.next()).await {
        Ok(Some(Ok(frame))) => Ok(NextFrame::Ready(frame)),
        Ok(Some(Err(error))) => Err(error),
        Ok(None) => Ok(NextFrame::Closed),
        Err(_) => Ok(NextFrame::TimedOut),
    }
}

/// Await `handling` while watching the reader for a peer disconnect.
///
/// At most one inbound frame is buffered while `handling` runs, so the
/// connection task never reads ahead arbitrarily. Once EOF or a read error is
/// observed, later calls dispatch the buffered frame without polling the
/// reader again.
pub(super) async fn handle_watching_peer<H, R, F>(
    handling: H,
    reader: &mut R,
    peer_closed: &CancellationToken,
    pending_frame: &mut Option<F>,
) -> io::Result<()>
where
    H: Future<Output = io::Result<()>>,
    R: Stream<Item = io::Result<F>> + Unpin,
{
    tokio::pin!(handling);
    if pending_frame.is_some() || peer_closed.is_cancelled() {
        return handling.await;
    }

    tokio::select! {
        biased;
        result = &mut handling => result,
        next = reader.next() => match next {
            Some(Ok(frame)) => {
                *pending_frame = Some(frame);
                handling.await
            }
            Some(Err(error)) => Err(error),
            None => {
                peer_closed.cancel();
                handling.await
            }
        },
    }
}
