//! Unit tests for connection actor scheduling and response installation.

mod completion;

use std::sync::{
    Arc,
    atomic::{AtomicUsize, Ordering},
};

use tokio::{
    sync::mpsc,
    time::{Duration, timeout},
};
use tokio_util::sync::CancellationToken;

use super::{
    ConnectionActor,
    ConnectionChannels,
    ResponseOutcome,
    event::Event,
    grace::{GraceFuture, GraceTimer},
    state::ActorState,
};
use crate::{
    app::Envelope,
    hooks::ProtocolHooks,
    push::PushQueues,
    response::{FrameStream, Response, WireframeError},
};

fn test_actor(
    hooks: ProtocolHooks<Envelope, ()>,
) -> Result<ConnectionActor<Envelope, ()>, crate::push::PushConfigError> {
    let (queues, handle) = PushQueues::<Envelope>::builder()
        .high_capacity(1)
        .low_capacity(1)
        .build()?;
    Ok(ConnectionActor::new_untracked(
        ConnectionChannels::new(queues, handle),
        None,
        CancellationToken::new(),
        hooks,
    ))
}

fn counting_hooks(calls: Arc<AtomicUsize>) -> ProtocolHooks<Envelope, ()> {
    ProtocolHooks {
        on_command_end: Some(Box::new(move |_ctx| {
            calls.fetch_add(1, Ordering::SeqCst);
        })),
        ..ProtocolHooks::default()
    }
}

#[tokio::test]
async fn shutdown_wins_over_a_ready_high_priority_push() {
    let (queues, handle) = PushQueues::<u8>::builder()
        .high_capacity(1)
        .low_capacity(1)
        .build()
        .expect("push queues should build");
    handle
        .push_high_priority(7)
        .await
        .expect("high-priority push should queue");

    let shutdown = CancellationToken::new();
    let mut actor: ConnectionActor<u8, crate::NoProtocolError> =
        ConnectionActor::new(queues, handle, None, shutdown.clone());
    shutdown.cancel();

    let event = actor.next_event(&ActorState::new(false, false)).await;

    assert!(
        matches!(event, Event::Shutdown),
        "shutdown should take precedence over a ready high-priority push"
    );
    assert_eq!(
        actor
            .high_rx
            .as_ref()
            .expect("high-priority queue should remain open")
            .len(),
        1,
        "shutdown precedence should leave the ready push queued"
    );
}

#[tokio::test]
async fn installs_single_response_with_correlation() {
    let calls = Arc::new(AtomicUsize::new(0));
    let mut actor = test_actor(counting_hooks(Arc::clone(&calls))).expect("actor should build");
    let mut sink = Vec::new();

    actor
        .install_response(
            Response::Single(Envelope::new(5, None, vec![1, 2])),
            Some(42),
            &mut sink,
        )
        .await
        .expect("single response should install");

    assert_eq!(sink.len(), 1, "single response must emit exactly one frame");
    let frame = sink.first().expect("frame should be present");
    assert_eq!(frame.id, 5, "frame identity must be preserved");
    assert_eq!(
        frame.correlation_id,
        Some(42),
        "request correlation must be stamped"
    );
    assert_eq!(frame.payload, vec![1, 2], "payload must be preserved");
    assert_eq!(
        calls.load(Ordering::SeqCst),
        1,
        "on_command_end must run once"
    );
}

#[tokio::test]
async fn preserves_frame_correlation_when_request_has_none() {
    let calls = Arc::new(AtomicUsize::new(0));
    let mut actor = test_actor(counting_hooks(Arc::clone(&calls))).expect("actor should build");
    let mut sink = Vec::new();

    actor
        .install_response(
            Response::Single(Envelope::new(5, Some(99), vec![1])),
            None,
            &mut sink,
        )
        .await
        .expect("single response should install");

    let frame = sink.first().expect("frame should be present");
    assert_eq!(
        frame.correlation_id,
        Some(99),
        "a request without a correlation id must not clear the frame's own"
    );
    assert_eq!(
        calls.load(Ordering::SeqCst),
        1,
        "on_command_end must run once"
    );
}

#[tokio::test]
async fn installs_vec_response_with_one_command_end() {
    let calls = Arc::new(AtomicUsize::new(0));
    let mut actor = test_actor(counting_hooks(Arc::clone(&calls))).expect("actor should build");
    let mut sink = Vec::new();

    actor
        .install_response(
            Response::Vec(vec![
                Envelope::new(1, None, vec![1]),
                Envelope::new(2, None, vec![2]),
            ]),
            Some(7),
            &mut sink,
        )
        .await
        .expect("vec response should install");

    assert_eq!(sink.len(), 2, "both frames must be emitted");
    assert!(
        sink.iter().all(|frame| frame.correlation_id == Some(7)),
        "every frame must carry the request correlation"
    );
    assert_eq!(
        calls.load(Ordering::SeqCst),
        1,
        "on_command_end must run once for the whole response"
    );
}

#[tokio::test]
async fn installs_empty_response_without_frames() {
    let calls = Arc::new(AtomicUsize::new(0));
    let mut actor = test_actor(counting_hooks(Arc::clone(&calls))).expect("actor should build");
    let mut sink = Vec::new();

    actor
        .install_response(Response::Empty, Some(3), &mut sink)
        .await
        .expect("empty response should install");

    assert!(sink.is_empty(), "empty response must emit no frames");
    assert_eq!(
        calls.load(Ordering::SeqCst),
        1,
        "on_command_end must still run once"
    );
}

#[tokio::test]
async fn rejects_single_response_while_multi_packet_is_active() {
    let calls = Arc::new(AtomicUsize::new(0));
    let mut actor = test_actor(counting_hooks(Arc::clone(&calls))).expect("actor should build");
    let (_tx, rx) = mpsc::channel(1);
    actor
        .set_multi_packet(Some(rx))
        .expect("multi-packet output should install");
    let mut sink = Vec::new();

    let error = actor
        .install_response(
            Response::Single(Envelope::new(1, None, vec![1])),
            Some(9),
            &mut sink,
        )
        .await
        .expect_err("an active output must reject a completed response");

    assert!(
        matches!(error, WireframeError::Io(ref io_error) if io_error.kind() == std::io::ErrorKind::InvalidInput),
        "state violations surface as invalid-input I/O errors"
    );
    assert!(sink.is_empty(), "a rejected response must emit nothing");
    assert_eq!(
        calls.load(Ordering::SeqCst),
        0,
        "a rejected response must not signal command completion"
    );
}

#[tokio::test]
async fn drives_stream_response_until_completion() {
    let calls = Arc::new(AtomicUsize::new(0));
    let mut actor = test_actor(counting_hooks(Arc::clone(&calls))).expect("actor should build");
    let stream: FrameStream<Envelope, ()> = Box::pin(futures::stream::iter(vec![
        Ok(Envelope::new(1, None, vec![1])),
        Ok(Envelope::new(2, None, vec![2])),
    ]));
    let mut sink = Vec::new();

    actor
        .install_response(Response::Stream(stream), Some(11), &mut sink)
        .await
        .expect("stream should drive to completion");

    let ids: Vec<u32> = sink.iter().map(|frame| frame.id).collect();
    assert_eq!(ids, vec![1, 2], "frames must be emitted in order");
    assert_eq!(
        calls.load(Ordering::SeqCst),
        1,
        "stream completion must signal exactly one command end"
    );
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

/// Deadline a test fires by hand instead of waiting for a clock.
struct ManualGraceTimer {
    /// Cancelled when the test decides the grace has elapsed.
    fired: CancellationToken,
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

#[tokio::test]
async fn drive_response_without_output_is_immediate() {
    let calls = Arc::new(AtomicUsize::new(0));
    let mut actor = test_actor(counting_hooks(Arc::clone(&calls))).expect("actor should build");
    let mut sink = Vec::new();

    timeout(Duration::from_millis(100), actor.drive_response(&mut sink))
        .await
        .expect("driving with no output must not wait")
        .expect("driving with no output must succeed");

    assert!(sink.is_empty(), "no frames should be emitted");
    assert_eq!(calls.load(Ordering::SeqCst), 0, "no command should end");
}
