//! Unit tests for peer observation while a response is driven.
//!
//! Each test drives [`handle_watching_peer`](super::handle_watching_peer) over
//! a synthetic reader and asserts which token the dispatch signals, so the
//! cancellation policy is pinned independently of a real transport.

use std::time::Duration;

use futures::stream;
use tokio::time::timeout;

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
/// observed. The connection-wide EOF token must stay live, because the next
/// response can be observed again.
#[tokio::test(start_paused = true)]
async fn buffered_frame_signals_an_unobserved_peer() -> io::Result<()> {
    let peer_eof = CancellationToken::new();
    let unobserved = peer_eof.child_token();
    let cause = OnceLock::new();
    let watcher = unobserved.clone();
    let handling = async move {
        watcher.cancelled().await;
        Ok(Dispatch::Continue)
    };
    let mut reader = stream::iter([Ok(1_u8)]);
    let mut pending = None;

    let waited = timeout(
        Duration::from_secs(120),
        handle_watching_peer(
            handling,
            &mut reader,
            PeerObservation {
                eof: &peer_eof,
                unobserved: &unobserved,
                cause: &cause,
            },
            &mut pending,
        ),
    )
    .await
    .map_err(|_| io::Error::other("a buffered frame pinned the response wait"))?;
    waited?;

    ensure(pending == Some(1), "the pipelined frame stays buffered")?;
    ensure(
        cause.get() == Some(&UnobservedCause::BufferedRequest),
        "the cancellation warning must name the buffered request",
    )?;
    ensure(
        unobserved.is_cancelled(),
        "the peer is unobservable for this response",
    )?;
    ensure(
        !peer_eof.is_cancelled(),
        "a buffered request must not end peer observation for the connection",
    )?;
    ensure(
        !peer_eof.child_token().is_cancelled(),
        "a later response must be observable again",
    )?;
    Ok(())
}

/// A frame buffered before this dispatch also stops observation: the read
/// half is not polled while it waits, so this dispatch cannot see a later
/// disconnect. The token that must be cancelled is the fresh one installed
/// for this dispatch; the dispatch that read the frame cancelled its own.
#[tokio::test(start_paused = true)]
async fn a_frame_buffered_before_the_dispatch_is_signalled() -> io::Result<()> {
    let peer_eof = CancellationToken::new();
    let unobserved = peer_eof.child_token();
    let cause = OnceLock::new();
    let watcher = unobserved.clone();
    let handling = async move {
        watcher.cancelled().await;
        Ok(Dispatch::Continue)
    };
    let mut reader = stream::pending::<io::Result<u8>>();
    let mut pending = Some(1_u8);

    let waited = timeout(
        Duration::from_secs(120),
        handle_watching_peer(
            handling,
            &mut reader,
            PeerObservation {
                eof: &peer_eof,
                unobserved: &unobserved,
                cause: &cause,
            },
            &mut pending,
        ),
    )
    .await
    .map_err(|_| io::Error::other("a buffered frame pinned the response wait"))?;
    waited?;

    ensure(pending == Some(1), "the pipelined frame stays buffered")?;
    ensure(
        cause.get() == Some(&UnobservedCause::BufferedRequest),
        "the cancellation warning must name the buffered request",
    )?;
    ensure(
        unobserved.is_cancelled(),
        "the response must be bounded for this dispatch",
    )?;
    ensure(
        !peer_eof.is_cancelled(),
        "a buffered request must not end peer observation for the connection",
    )?;
    Ok(())
}

/// A clean EOF also stops observation so the response wait is bounded, and
/// it is permanent: later responses inherit the cancelled token.
#[tokio::test(start_paused = true)]
async fn clean_eof_signals_an_unobserved_peer() -> io::Result<()> {
    let peer_eof = CancellationToken::new();
    let unobserved = peer_eof.child_token();
    let cause = OnceLock::new();
    let watcher = unobserved.clone();
    let handling = async move {
        watcher.cancelled().await;
        Ok(Dispatch::Continue)
    };
    let mut reader = stream::iter([]);
    let mut pending: Option<u8> = None;

    let waited = timeout(
        Duration::from_secs(120),
        handle_watching_peer(
            handling,
            &mut reader,
            PeerObservation {
                eof: &peer_eof,
                unobserved: &unobserved,
                cause: &cause,
            },
            &mut pending,
        ),
    )
    .await
    .map_err(|_| io::Error::other("EOF pinned the response wait"))?;
    waited?;

    ensure(pending.is_none(), "EOF buffers no frame")?;
    ensure(
        cause.get() == Some(&UnobservedCause::HalfClose),
        "the cancellation warning must name the half-close",
    )?;
    ensure(unobserved.is_cancelled(), "the response is bounded")?;
    ensure(peer_eof.is_cancelled(), "the peer is marked closed")?;
    ensure(
        peer_eof.child_token().is_cancelled(),
        "a response after EOF inherits the closed peer",
    )?;
    Ok(())
}

/// A read error fails the connection without marking the peer unobservable;
/// the error itself is what ends the connection.
#[tokio::test(start_paused = true)]
async fn read_error_fails_the_connection() -> io::Result<()> {
    let peer_eof = CancellationToken::new();
    let unobserved = peer_eof.child_token();
    let cause = OnceLock::new();
    let handling = std::future::pending::<io::Result<Dispatch>>();
    let mut reader = stream::iter([Err(io::Error::other("peer read failed"))]);
    let mut pending: Option<u8> = None;

    let error = handle_watching_peer(
        handling,
        &mut reader,
        PeerObservation {
            eof: &peer_eof,
            unobserved: &unobserved,
            cause: &cause,
        },
        &mut pending,
    )
    .await
    .expect_err("a read error must fail the connection");

    ensure(
        error.to_string().contains("peer read failed"),
        "the read error must be propagated",
    )?;
    ensure(
        cause.get().is_none(),
        "a read error is not a peer-observation cause",
    )?;
    ensure(
        !unobserved.is_cancelled(),
        "no response token was cancelled",
    )?;
    ensure(!peer_eof.is_cancelled(), "the peer is not marked closed")?;
    Ok(())
}
