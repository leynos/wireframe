//! Idle-grace cancellation for a response whose peer is unobservable.

use std::{
    future::Future,
    io,
    sync::{
        Arc,
        Mutex,
        PoisonError,
        atomic::{AtomicUsize, Ordering},
    },
    time::Duration,
};

use tokio::time::timeout;
use tokio_util::sync::CancellationToken;

use super::{test_actor, test_actor_with_shutdown};
use crate::{
    app::Envelope,
    connection::{
        FrameSink,
        ProgressSignal,
        ResponseOutcome,
        grace::{GraceFuture, GraceTimer},
    },
    hooks::ProtocolHooks,
    response::{FrameStream, Response, WireframeError},
};

/// Sink that accepts every frame, but only after a delay.
///
/// A write sink that is slow rather than stalled separates a peer that is still
/// reading from one that has stopped, which is what the idle grace must
/// distinguish.
struct PacedSink {
    /// How long each write takes to be accepted.
    delay: Duration,
    /// Frames the peer accepted, recorded when their write completed.
    written: Arc<Mutex<Vec<Envelope>>>,
}

impl FrameSink<Envelope> for PacedSink {
    fn send_frame(&mut self, frame: Envelope) -> impl Future<Output = io::Result<()>> + Send {
        let delay = self.delay;
        let written = Arc::clone(&self.written);
        async move {
            tokio::time::sleep(delay).await;
            // A sink that only ever pushes cannot observe a torn record, so a
            // poisoned lock still holds a usable value.
            written
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .push(frame);
            Ok(())
        }
    }
}

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

/// A batch whose frames are each accepted within the grace is delivered.
///
/// Writing the whole batch takes longer than one grace, so delivery proves the
/// bound is renewed after each accepted frame rather than applied once to the
/// batch. A peer that keeps reading is still making progress and must not lose
/// the rest of its output.
#[tokio::test(start_paused = true)]
async fn a_batch_renews_the_grace_after_each_accepted_frame() {
    const GRACE: Duration = Duration::from_secs(5);
    const PACING: Duration = Duration::from_secs(4);
    const FRAMES: u32 = 3;

    let mut actor = test_actor(ProtocolHooks::default()).expect("actor should build");
    actor.set_peer_closed_grace(GRACE);
    let peer_closed = CancellationToken::new();
    actor.set_peer_closed(peer_closed.clone());
    let written = Arc::new(Mutex::new(Vec::new()));
    let mut sink = PacedSink {
        delay: PACING,
        written: Arc::clone(&written),
    };
    let frames = (0..FRAMES)
        .map(|id| Envelope::new(id, None, Vec::new()))
        .collect();

    peer_closed.cancel();
    let outcome = timeout(
        Duration::from_secs(60),
        actor.install_response(Response::Vec(frames), Some(7), &mut sink),
    )
    .await
    .expect("each frame is accepted within its own grace")
    .expect("a progressing batch is not an error");

    assert_eq!(outcome, ResponseOutcome::Completed);
    let ids: Vec<u32> = written
        .lock()
        .expect("the sink record must not be poisoned")
        .iter()
        .map(|frame| frame.id)
        .collect();
    assert_eq!(
        ids,
        (0..FRAMES).collect::<Vec<_>>(),
        "every frame must be written in emission order"
    );
}

/// Sink that writes one composed frame as several paced physical writes.
///
/// This is the shape a fragmenting transport presents to the actor: one future
/// per logical frame, several writes inside it, and a progress report after
/// each write reaches the transport.
struct FragmentingSink {
    /// How long each physical write takes to be accepted.
    delay: Duration,
    /// Physical writes one composed frame is split into.
    fragments: usize,
    /// Physical writes accepted so far.
    written: Arc<AtomicUsize>,
    /// Signal raised after each accepted write.
    progress: Arc<ProgressSignal>,
}

impl FrameSink<Envelope> for FragmentingSink {
    fn send_frame(&mut self, _frame: Envelope) -> impl Future<Output = io::Result<()>> + Send {
        let delay = self.delay;
        let fragments = self.fragments;
        let written = Arc::clone(&self.written);
        let progress = Arc::clone(&self.progress);
        async move {
            for _ in 0..fragments {
                tokio::time::sleep(delay).await;
                written.fetch_add(1, Ordering::SeqCst);
                progress.report();
            }
            Ok(())
        }
    }

    fn progress(&self) -> Option<Arc<ProgressSignal>> { Some(Arc::clone(&self.progress)) }
}

/// A fragmented frame renews the grace after each accepted physical write.
///
/// One logical frame takes longer than a whole grace to transmit because each
/// of its fragments is accepted slowly, but no single write stalls. Delivery
/// proves the progress signal renews the deadline per physical write; bounding
/// the frame as a whole would cancel it part way through and close a response
/// the peer was still reading.
#[tokio::test(start_paused = true)]
async fn a_fragmented_frame_renews_the_grace_per_accepted_fragment() {
    const GRACE: Duration = Duration::from_secs(5);
    const PACING: Duration = Duration::from_secs(4);
    const FRAGMENTS: usize = 3;

    let mut actor = test_actor(ProtocolHooks::default()).expect("actor should build");
    actor.set_peer_closed_grace(GRACE);
    let peer_closed = CancellationToken::new();
    actor.set_peer_closed(peer_closed.clone());
    let written = Arc::new(AtomicUsize::new(0));
    let mut sink = FragmentingSink {
        delay: PACING,
        fragments: FRAGMENTS,
        written: Arc::clone(&written),
        progress: Arc::new(ProgressSignal::default()),
    };

    peer_closed.cancel();
    let outcome = timeout(
        Duration::from_secs(60),
        actor.install_response(
            Response::Vec(vec![Envelope::new(1, None, Vec::new())]),
            Some(7),
            &mut sink,
        ),
    )
    .await
    .expect("each fragment is accepted within its own grace")
    .expect("a progressing frame is not an error");

    assert_eq!(outcome, ResponseOutcome::Completed);
    assert_eq!(
        written.load(Ordering::SeqCst),
        FRAGMENTS,
        "every fragment of the frame must be written"
    );
}

/// A server stop must not let a silent producer pin the connection.
///
/// A server connection keeps draining a response whose peer is still reading,
/// so shutdown does not abandon it. A producer that yields nothing after the
/// stop is the case where that policy would otherwise hold the task, its gauge
/// guard, and the server drain until the peer goes away. The idle grace is what
/// releases it.
#[tokio::test(start_paused = true)]
async fn a_silent_producer_is_released_after_a_stop() {
    const GRACE: Duration = Duration::from_secs(5);

    let shutdown = CancellationToken::new();
    let mut actor = test_actor_with_shutdown(shutdown.clone(), ProtocolHooks::default())
        .expect("actor should build");
    actor.set_abandon_response_on_shutdown(false);
    actor.set_peer_closed_grace(GRACE);
    let stalled: FrameStream<Envelope, ()> = Box::pin(futures::stream::pending::<
        Result<Envelope, WireframeError<()>>,
    >());
    let mut sink = Vec::new();

    shutdown.cancel();
    let outcome = timeout(
        Duration::from_secs(10),
        actor.install_response(Response::Stream(stalled), Some(7), &mut sink),
    )
    .await
    .expect("the idle grace must bound the drain after a stop")
    .expect("a cancelled response is not an error");

    assert_eq!(outcome, ResponseOutcome::Cancelled);
    assert!(sink.is_empty(), "a cancelled response writes no frames");
}

/// A response still making progress after a stop is still delivered.
///
/// The whole response takes longer than one grace, so completion proves the
/// bounded drain renews its deadline per event. Applying one deadline to the
/// whole response would truncate output the peer is still able to read.
#[tokio::test(start_paused = true)]
async fn a_response_that_progresses_after_a_stop_is_delivered() {
    const GRACE: Duration = Duration::from_secs(5);
    const PACING: Duration = Duration::from_secs(4);
    const FRAMES: u32 = 3;

    let shutdown = CancellationToken::new();
    let mut actor = test_actor_with_shutdown(shutdown.clone(), ProtocolHooks::default())
        .expect("actor should build");
    actor.set_abandon_response_on_shutdown(false);
    actor.set_peer_closed_grace(GRACE);
    let stream: FrameStream<Envelope, ()> =
        Box::pin(futures::stream::unfold(0u32, |index| async move {
            if index == FRAMES {
                return None;
            }
            tokio::time::sleep(PACING).await;
            Some((Ok(Envelope::new(index, None, Vec::new())), index + 1))
        }));
    let mut sink = Vec::new();

    shutdown.cancel();
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
