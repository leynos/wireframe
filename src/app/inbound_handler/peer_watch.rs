//! Peer-disconnect observation for in-flight handler responses.
//!
//! A handler may stream for an unbounded time. Waiting for it before reading
//! the next inbound frame would hide a peer that has already gone away, leaving
//! the connection task, its gauge guard, and the server drain pending forever.
//! These helpers poll the read half while one frame is being handled.
//!
//! Two tokens carry the peer's state, and the distinction matters:
//!
//! - `peer_eof` is connection-wide and is cancelled only by a clean EOF. That state is permanent:
//!   the read half has ended, so no later response can be watched either.
//! - `unobserved` is per dispatch. The connection loop installs a fresh child of `peer_eof` on the
//!   actor before each frame, so cancelling it bounds one response without affecting the next.
//!
//! Cancellation policy:
//!
//! - A read error cancels the in-flight response and fails the connection.
//! - A clean EOF means the peer half-closed its write side. It may still be reading, so the
//!   response is allowed to finish; cancelling `unobserved` lets the actor bound how long it waits
//!   for a producer that never yields again.
//! - Once an inbound frame is buffered, the read half is no longer polled, so a later disconnect
//!   cannot be observed directly. Only that response is signalled as unobservable, and the actor
//!   bounds each idle wait rather than waiting forever for a producer that never yields again.
//! - If that bound expires, the response is cancelled without a terminator. The loop then stops
//!   dispatching: the buffered request is dropped, because a later response would be read as part
//!   of a stream the client still considers open.

use std::{future::Future, io, pin::Pin, sync::OnceLock};

use futures::{Stream, StreamExt};
use log::debug;
use tokio_util::sync::CancellationToken;

use super::super::frame_handling::Dispatch;
use crate::connection::UnobservedCause;

/// The two peer-state tokens one dispatch observes.
///
/// They are kept together because their lifetimes differ: `eof` outlives the
/// connection, while `unobserved` belongs to the response being driven.
pub(super) struct PeerObservation<'a> {
    /// Connection-wide token, cancelled only by a clean EOF.
    pub(super) eof: &'a CancellationToken,
    /// Per-dispatch token the actor watches for this response.
    pub(super) unobserved: &'a CancellationToken,
    /// Records why direct observation stopped, for the cancellation warning.
    pub(super) cause: &'a OnceLock<UnobservedCause>,
}

impl<'a> PeerObservation<'a> {
    /// Bundle the tokens and cause cell one dispatch observes.
    pub(super) const fn new(
        eof: &'a CancellationToken,
        unobserved: &'a CancellationToken,
        cause: &'a OnceLock<UnobservedCause>,
    ) -> Self {
        Self {
            eof,
            unobserved,
            cause,
        }
    }
}

/// Await `handling` while watching the reader for a peer disconnect.
///
/// At most one inbound frame is buffered while `handling` runs, so the
/// connection task never reads ahead arbitrarily. Once EOF or a read error is
/// observed, later calls dispatch without polling the reader again, and they
/// still mark the peer unobservable so every later response stays bounded.
pub(super) async fn handle_watching_peer<H, R, F>(
    handling: H,
    reader: &mut R,
    observation: PeerObservation<'_>,
    pending_frame: &mut Option<F>,
) -> io::Result<Dispatch>
where
    H: Future<Output = io::Result<Dispatch>>,
    R: Stream<Item = io::Result<F>> + Unpin,
{
    tokio::pin!(handling);
    // A buffered frame means the read half is not polled again while it waits,
    // so this dispatch cannot observe a later disconnect. The token to signal is
    // the one installed for *this* dispatch, which is fresh: the dispatch that
    // read the frame cancelled its own token, not this one.
    if pending_frame.is_some() {
        return await_unobserved_peer(
            handling.as_mut(),
            observation.unobserved,
            observation.cause,
            UnobservedCause::BufferedRequest,
        )
        .await;
    }
    // An EOF seen earlier is permanent, but it still has to be signalled for
    // *this* response: a drain entered after the EOF would otherwise wait on a
    // producer nobody can interrupt. Signalling is idempotent, so a dispatch
    // that follows the EOF-driven drain re-cancels a token that is already
    // cancelled.
    if observation.eof.is_cancelled() {
        return await_unobserved_peer(
            handling.as_mut(),
            observation.unobserved,
            observation.cause,
            UnobservedCause::HalfClose,
        )
        .await;
    }

    tokio::select! {
        biased;
        result = &mut handling => result,
        next = reader.next() => {
            settle_read(next, pending_frame, handling.as_mut(), observation).await
        }
    }
}

/// Decide what the read half's outcome means for the in-flight response.
///
/// A buffered frame ends direct observation, because the read half is not
/// polled again while that frame waits. A clean EOF is recorded connection-wide
/// first, so every later response inherits it. Both then bound the response
/// wait through the actor; a read error simply fails the connection.
async fn settle_read<H, F>(
    next: Option<io::Result<F>>,
    pending_frame: &mut Option<F>,
    handling: Pin<&mut H>,
    observation: PeerObservation<'_>,
) -> io::Result<Dispatch>
where
    H: Future<Output = io::Result<Dispatch>>,
{
    let PeerObservation {
        eof,
        unobserved,
        cause,
    } = observation;
    match next {
        Some(Ok(frame)) => {
            *pending_frame = Some(frame);
            await_unobserved_peer(
                handling,
                unobserved,
                cause,
                UnobservedCause::BufferedRequest,
            )
            .await
        }
        Some(Err(error)) => Err(error),
        None => {
            eof.cancel();
            await_unobserved_peer(handling, unobserved, cause, UnobservedCause::HalfClose).await
        }
    }
}

/// Stop observing the peer's read half and await the response.
///
/// Both a clean EOF and a buffered inbound frame end direct observation: the
/// read half is either at its end or deliberately not polled again. Cancelling
/// the per-dispatch `unobserved` token hands the wait to the actor's bounded
/// drain, which still delivers frames from a producer that is merely slow, but
/// releases the connection once a producer stops yielding. The
/// connection-wide EOF token is never touched here.
///
/// # Examples
///
/// ```ignore
/// let handling = drive_response(&mut actor, &mut sink);
/// tokio::pin!(handling);
/// await_unobserved_peer(handling.as_mut(), &unobserved, UnobservedCause::HalfClose).await?;
/// ```
async fn await_unobserved_peer<H>(
    handling: Pin<&mut H>,
    unobserved: &CancellationToken,
    cause_cell: &OnceLock<UnobservedCause>,
    cause: UnobservedCause,
) -> io::Result<Dispatch>
where
    H: Future<Output = io::Result<Dispatch>>,
{
    debug!(
        "stopping peer observation after {}; the actor bounds the pending response",
        cause.as_str()
    );
    // Recorded before the token is cancelled: the actor reads it when the
    // grace expires and names the cause in its cancellation warning.
    cause_cell.set(cause).ok();
    unobserved.cancel();
    handling.await
}

#[cfg(test)]
#[path = "peer_watch_tests.rs"]
mod tests;
