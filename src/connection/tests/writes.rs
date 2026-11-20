//! Abandoning a write the peer never accepts.

use std::{
    future::Future,
    io,
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    },
    time::Duration,
};

use tokio_util::sync::CancellationToken;

use super::{counting_hooks, grace::ManualGraceTimer, test_actor, test_actor_with_shutdown};
use crate::{
    app::Envelope,
    connection::{FrameSink, PushSink, ResponseOutcome, WriteBound, write_before_bound},
    hooks::ProtocolHooks,
    response::{FrameStream, Response},
};

/// Sink whose writes never complete, modelling a peer that stopped reading.
struct StalledSink;

impl FrameSink<Envelope> for StalledSink {
    fn send_frame(&mut self, _frame: Envelope) -> impl Future<Output = io::Result<()>> + Send {
        std::future::pending()
    }
}

impl PushSink<Envelope> for StalledSink {
    fn send_push(&mut self, _frame: Envelope) -> impl Future<Output = io::Result<()>> + Send {
        std::future::pending()
    }
}

/// A response with one frame ready to write.
fn one_frame_response() -> Response<Envelope, ()> {
    let stream: FrameStream<Envelope, ()> = Box::pin(futures::stream::iter(vec![Ok(
        Envelope::new(1, None, vec![1]),
    )]));
    Response::Stream(stream)
}

/// A pending write must be abandoned when the server shuts down.
///
/// A peer that stops reading applies back-pressure that can hold a write open
/// indefinitely. Without bounding it, a graceful server drain would wait for a
/// peer that may never read again.
#[tokio::test]
async fn shutdown_abandons_a_write_the_peer_never_accepts() {
    let shutdown = CancellationToken::new();
    let mut actor = test_actor_with_shutdown(shutdown.clone(), ProtocolHooks::default())
        .expect("actor should build");
    let mut sink = StalledSink;

    let outcome = {
        let mut driving =
            std::pin::pin!(actor.install_response(one_frame_response(), Some(7), &mut sink));
        assert!(
            futures::poll!(driving.as_mut()).is_pending(),
            "the write must be pending before shutdown is requested"
        );

        shutdown.cancel();
        driving
            .await
            .expect("an abandoned response is not an error")
    };

    assert_eq!(outcome, ResponseOutcome::Cancelled);
}

/// The idle grace must also bound a write the peer never accepts.
///
/// The grace bounds the wait for the next event, so a write left outside it
/// would still let a vanished peer pin the connection and the server drain.
/// Both orderings matter: the peer may already be unobservable when the write
/// starts, or observation may end while the write is still pending. The second
/// is what a peer that closes its input half and then stops reading looks like.
#[rstest::rstest]
#[case::peer_already_closed(true)]
#[case::peer_closes_during_the_write(false)]
#[tokio::test]
async fn the_grace_abandons_a_write_the_peer_never_accepts(#[case] close_peer_before_write: bool) {
    let fired = CancellationToken::new();
    let mut actor = test_actor(ProtocolHooks::default()).expect("actor should build");
    actor.set_grace_timer(Arc::new(ManualGraceTimer {
        fired: fired.clone(),
    }));
    let peer_closed = CancellationToken::new();
    actor.set_peer_closed(peer_closed.clone());
    let mut sink = StalledSink;

    if close_peer_before_write {
        peer_closed.cancel();
    }
    let outcome = {
        let mut driving =
            std::pin::pin!(actor.install_response(one_frame_response(), Some(7), &mut sink));
        assert!(
            futures::poll!(driving.as_mut()).is_pending(),
            "the write must be pending before the grace fires"
        );

        if !close_peer_before_write {
            peer_closed.cancel();
            assert!(
                futures::poll!(driving.as_mut()).is_pending(),
                "a write pending at the half-close must survive the grace itself"
            );
        }

        fired.cancel();
        driving
            .await
            .expect("an abandoned response is not an error")
    };

    assert_eq!(outcome, ResponseOutcome::Cancelled);
}

/// A ready write is preferred over a bound that has already fired.
///
/// A cancellation observed while the transport still accepts frames must not
/// discard output the peer can receive; only a write that would block is
/// abandoned.
#[tokio::test]
async fn a_ready_write_wins_over_an_expired_bound() {
    let shutdown = CancellationToken::new();
    shutdown.cancel();

    let bound = write_before_bound(async { Ok(()) }, &shutdown, std::future::pending::<()>())
        .await
        .expect("a ready write is not an error");

    assert_eq!(
        bound,
        WriteBound::Finished,
        "a write the transport accepts must not be abandoned"
    );
}

/// A blocked write is abandoned once the server stops.
#[tokio::test]
async fn a_blocked_write_is_abandoned_by_shutdown() {
    let shutdown = CancellationToken::new();
    shutdown.cancel();

    let bound = write_before_bound(
        std::future::pending::<io::Result<()>>(),
        &shutdown,
        std::future::pending::<()>(),
    )
    .await
    .expect("abandoning a write is not an error");

    assert_eq!(
        bound,
        WriteBound::Stopped,
        "shutdown must abandon a write that cannot complete"
    );
}

/// The idle grace abandons a blocked write when the interval elapses.
///
/// The clock is paused so the assertion proves the write ends at the deadline
/// rather than merely eventually.
#[tokio::test(start_paused = true)]
async fn a_blocked_write_is_abandoned_by_the_grace() {
    let shutdown = CancellationToken::new();
    let started = tokio::time::Instant::now();

    let bound = write_before_bound(
        std::future::pending::<io::Result<()>>(),
        &shutdown,
        tokio::time::sleep(Duration::from_secs(30)),
    )
    .await
    .expect("abandoning a write is not an error");

    assert_eq!(
        bound,
        WriteBound::GraceExpired,
        "the grace must abandon a write the peer never accepts"
    );
    assert_eq!(
        started.elapsed(),
        Duration::from_secs(30),
        "the write must be abandoned exactly when the grace expires"
    );
}

/// A write that is still pending and inside both bounds keeps waiting.
#[tokio::test(start_paused = true)]
async fn a_pending_write_waits_while_no_bound_fires() {
    let shutdown = CancellationToken::new();
    let waiting = write_before_bound(
        std::future::pending::<io::Result<()>>(),
        &shutdown,
        tokio::time::sleep(Duration::from_secs(5)),
    );
    tokio::pin!(waiting);

    assert!(
        futures::poll!(waiting.as_mut()).is_pending(),
        "neither bound has fired, so the write must keep waiting"
    );
}

/// A transport failure is reported rather than mistaken for abandonment.
#[tokio::test]
async fn a_transport_error_is_reported() {
    let shutdown = CancellationToken::new();

    let error = write_before_bound(
        async { Err(io::Error::other("broken pipe")) },
        &shutdown,
        std::future::pending::<()>(),
    )
    .await
    .expect_err("the transport error must be reported");

    assert_eq!(
        error.kind(),
        io::ErrorKind::Other,
        "the transport's own error must reach the caller"
    );
}

/// A single-frame response write must also be abandonable.
///
/// `Single` and `Vec` responses look small, but one frame is enough to block
/// on a peer that stopped reading, so their write is bounded like a stream
/// write. A cancelled write reports no completion, because the connection is
/// closing and the response was not fully delivered.
#[tokio::test]
async fn shutdown_abandons_a_single_frame_response_write() {
    let shutdown = CancellationToken::new();
    let calls = Arc::new(AtomicUsize::new(0));
    let mut actor = test_actor_with_shutdown(shutdown.clone(), counting_hooks(Arc::clone(&calls)))
        .expect("actor should build");
    let mut sink = StalledSink;

    let outcome = {
        let mut driving = std::pin::pin!(actor.install_response(
            Response::Single(Envelope::new(1, None, vec![1])),
            Some(7),
            &mut sink,
        ));
        assert!(
            futures::poll!(driving.as_mut()).is_pending(),
            "the write must be pending before shutdown is requested"
        );

        shutdown.cancel();
        driving
            .await
            .expect("an abandoned response is not an error")
    };

    assert_eq!(outcome, ResponseOutcome::Cancelled);
    assert_eq!(
        calls.load(Ordering::SeqCst),
        0,
        "an abandoned response must not run the completion hook"
    );
}
