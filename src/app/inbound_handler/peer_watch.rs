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
//! - Once an inbound frame is buffered, the read half is no longer polled, so a later disconnect
//!   cannot be observed directly. The peer is then signalled as unobservable, and the actor bounds
//!   each idle wait instead of waiting forever for a producer that never yields again.

use std::{future::Future, io, pin::Pin};

use futures::{Stream, StreamExt};
use log::debug;
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
    // A buffered frame means the read half is no longer polled; the token was
    // cancelled when it was buffered, so the actor already bounds idle waits.
    if pending_frame.is_some() || peer_closed.is_cancelled() {
        return handling.await;
    }

    tokio::select! {
        biased;
        result = &mut handling => result,
        next = reader.next() => match next {
            Some(Ok(frame)) => {
                *pending_frame = Some(frame);
                await_unobserved_peer(handling.as_mut(), peer_closed, UnobservedCause::BufferedRequest)
                    .await
            }
            Some(Err(error)) => Err(error),
            None => {
                await_unobserved_peer(handling.as_mut(), peer_closed, UnobservedCause::HalfClose).await
            }
        },
    }
}

/// Why direct peer observation stopped.
#[derive(Clone, Copy, Debug)]
enum UnobservedCause {
    /// The peer closed its input half.
    HalfClose,
    /// An inbound frame was buffered for the next request.
    BufferedRequest,
}

impl UnobservedCause {
    /// Describe the cause for the peer-observation diagnostic.
    const fn as_str(self) -> &'static str {
        match self {
            Self::HalfClose => "a half-close",
            Self::BufferedRequest => "a buffered request",
        }
    }
}

/// Stop observing the peer's read half and await the response.
///
/// Both a clean EOF and a buffered inbound frame end direct observation: the
/// read half is either at its end or deliberately not polled again. Cancelling
/// `peer_closed` hands the wait to the actor's bounded drain, which still
/// delivers frames from a producer that is merely slow, but releases the
/// connection once a producer stops yielding.
///
/// # Examples
///
/// ```ignore
/// let handling = drive_response(&mut actor, &mut sink);
/// tokio::pin!(handling);
/// await_unobserved_peer(handling.as_mut(), &peer_closed, UnobservedCause::HalfClose).await?;
/// ```
async fn await_unobserved_peer<H>(
    handling: Pin<&mut H>,
    peer_closed: &CancellationToken,
    cause: UnobservedCause,
) -> io::Result<()>
where
    H: Future<Output = io::Result<()>>,
{
    debug!(
        "stopping peer observation after {}; the actor bounds the pending response",
        cause.as_str()
    );
    peer_closed.cancel();
    handling.await
}

#[cfg(test)]
mod tests {
    //! Unit tests for peer observation while a response is driven.

    use futures::stream;

    use super::*;

    /// Require `condition`, reporting `message` as the test failure otherwise.
    ///
    /// These tests return `io::Result` so that a failed expectation is a
    /// returned error rather than a panic inside a `Result`-returning function.
    fn ensure(condition: bool, message: &str) -> io::Result<()> {
        if condition {
            Ok(())
        } else {
            Err(io::Error::other(message.to_owned()))
        }
    }

    /// A buffered frame stops the read half being polled, so the actor must be
    /// told to bound the response instead of waiting for a peer that cannot be
    /// observed.
    #[tokio::test(start_paused = true)]
    async fn buffered_frame_signals_an_unobserved_peer() -> io::Result<()> {
        let peer_closed = CancellationToken::new();
        let watcher = peer_closed.clone();
        let handling = async move {
            watcher.cancelled().await;
            Ok(())
        };
        let mut reader = stream::iter([Ok(1_u8)]);
        let mut pending = None;

        let waited = timeout(
            Duration::from_secs(120),
            handle_watching_peer(handling, &mut reader, &peer_closed, &mut pending),
        )
        .await
        .map_err(|_| io::Error::other("a buffered frame pinned the response wait"))?;
        waited?;

        ensure(pending == Some(1), "the pipelined frame stays buffered")?;
        ensure(
            peer_closed.is_cancelled(),
            "the peer is marked unobservable",
        )?;
        Ok(())
    }

    /// A clean EOF also stops observation so the response wait is bounded.
    #[tokio::test(start_paused = true)]
    async fn clean_eof_signals_an_unobserved_peer() -> io::Result<()> {
        let peer_closed = CancellationToken::new();
        let watcher = peer_closed.clone();
        let handling = async move {
            watcher.cancelled().await;
            Ok(())
        };
        let mut reader = stream::iter([]);
        let mut pending: Option<u8> = None;

        let waited = timeout(
            Duration::from_secs(120),
            handle_watching_peer(handling, &mut reader, &peer_closed, &mut pending),
        )
        .await
        .map_err(|_| io::Error::other("EOF pinned the response wait"))?;
        waited?;

        ensure(pending.is_none(), "EOF buffers no frame")?;
        ensure(peer_closed.is_cancelled(), "the peer is marked closed")?;
        Ok(())
    }
}
