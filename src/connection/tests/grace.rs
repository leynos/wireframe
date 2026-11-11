//! Idle-grace cancellation for a response whose peer is unobservable.

use std::{sync::Arc, time::Duration};

use tokio::time::timeout;
use tokio_util::sync::CancellationToken;

use super::test_actor;
use crate::{
    app::Envelope,
    connection::{
        ResponseOutcome,
        grace::{GraceFuture, GraceTimer},
    },
    hooks::ProtocolHooks,
    response::{FrameStream, Response, WireframeError},
};

/// Once the peer is unobservable, a producer that stops yielding is released
/// after the configured grace rather than pinning the connection.
#[tokio::test(start_paused = true)]
async fn a_stalled_response_is_cancelled_after_the_idle_grace() {
    let mut actor = test_actor(ProtocolHooks::default()).expect("actor should build");
    actor.set_peer_closed_grace(Duration::from_secs(5));
    let peer_closed = CancellationToken::new();
    actor.set_peer_closed(peer_closed.clone());
    let stalled: FrameStream<Envelope, ()> = Box::pin(futures::stream::pending::<
        Result<Envelope, WireframeError<()>>,
    >());
    let mut sink = Vec::new();

    peer_closed.cancel();
    let outcome = timeout(
        Duration::from_secs(10),
        actor.install_response(Response::Stream(stalled), Some(7), &mut sink),
    )
    .await
    .expect("the idle grace must bound the wait")
    .expect("a cancelled response is not an error");

    assert_eq!(outcome, ResponseOutcome::Cancelled);
    assert!(sink.is_empty(), "a cancelled response writes no frames");
}

/// A half-close must not truncate a response that keeps making progress.
///
/// The producer pauses for less than the grace before each frame, but the
/// whole response takes longer than one grace interval. Delivery therefore
/// proves the deadline is renewed per event rather than applied once to the
/// response, which is what keeps a valid half-close working.
#[tokio::test(start_paused = true)]
async fn a_response_that_progresses_after_a_half_close_is_delivered() {
    const GRACE: Duration = Duration::from_secs(5);
    const PACING: Duration = Duration::from_secs(2);
    const FRAMES: u32 = 3;

    let mut actor = test_actor(ProtocolHooks::default()).expect("actor should build");
    actor.set_peer_closed_grace(GRACE);
    let peer_closed = CancellationToken::new();
    actor.set_peer_closed(peer_closed.clone());
    let stream: FrameStream<Envelope, ()> =
        Box::pin(futures::stream::unfold(0u32, |index| async move {
            if index == FRAMES {
                return None;
            }
            tokio::time::sleep(PACING).await;
            let frame = Envelope::new(index, None, Vec::new());
            Some((Ok(frame), index + 1))
        }));
    let mut sink = Vec::new();

    peer_closed.cancel();
    let outcome = timeout(
        Duration::from_secs(60),
        actor.install_response(Response::Stream(stream), Some(7), &mut sink),
    )
    .await
    .expect("progress within each grace must complete the response")
    .expect("a progressing response is not an error");

    assert_eq!(outcome, ResponseOutcome::Completed);
    let ids: Vec<u32> = sink.iter().map(|frame| frame.id).collect();
    assert_eq!(
        ids,
        (0..FRAMES).collect::<Vec<_>>(),
        "every frame produced within the grace must be written"
    );
}

/// Deadline a test fires by hand instead of waiting for a clock.
pub(super) struct ManualGraceTimer {
    /// Cancelled when the test decides the grace has elapsed.
    pub(super) fired: CancellationToken,
}

impl GraceTimer for ManualGraceTimer {
    fn elapse(&self, _grace: Duration) -> GraceFuture {
        let fired = self.fired.clone();
        Box::pin(async move { fired.cancelled().await })
    }
}

/// The idle grace comes from the injected deadline source.
///
/// Firing the deadline proves cancellation waits for the grace and then
/// happens immediately, without depending on the wall clock.
#[tokio::test]
async fn a_stalled_response_is_cancelled_when_the_injected_grace_fires() {
    let fired = CancellationToken::new();
    let mut actor = test_actor(ProtocolHooks::default()).expect("actor should build");
    actor.set_grace_timer(Arc::new(ManualGraceTimer {
        fired: fired.clone(),
    }));
    let peer_closed = CancellationToken::new();
    actor.set_peer_closed(peer_closed.clone());
    let stalled: FrameStream<Envelope, ()> = Box::pin(futures::stream::pending::<
        Result<Envelope, WireframeError<()>>,
    >());
    let mut sink = Vec::new();

    peer_closed.cancel();
    let outcome = {
        // The pinned future borrows the sink, so it is scoped to release that
        // borrow before the sink is inspected.
        let mut driving =
            std::pin::pin!(actor.install_response(Response::Stream(stalled), Some(7), &mut sink));
        assert!(
            futures::poll!(driving.as_mut()).is_pending(),
            "an unfired grace must not cancel the response"
        );

        fired.cancel();
        driving.await.expect("a cancelled response is not an error")
    };

    assert_eq!(outcome, ResponseOutcome::Cancelled);
    assert!(sink.is_empty(), "a cancelled response writes no frames");
}
