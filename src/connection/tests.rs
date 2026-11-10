//! Unit tests for connection actor scheduling and response installation.

use std::{
    future::Future,
    io,
    sync::{
        Arc,
        Mutex,
        PoisonError,
        atomic::{AtomicUsize, Ordering},
    },
};

use rstest::rstest;
use tokio::{
    sync::mpsc,
    time::{Duration, timeout},
};
use tokio_util::sync::CancellationToken;

use super::{ConnectionActor, ConnectionChannels, FrameSink, event::Event, state::ActorState};
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

/// Sink that records each successful write in a shared event log.
struct RecordingSink {
    events: Arc<Mutex<Vec<&'static str>>>,
}

impl FrameSink<Envelope> for RecordingSink {
    // Recording cannot block, so the future is returned ready rather than
    // declared `async`; a needless `async` is rejected by the project lints.
    fn send_frame(&mut self, _frame: Envelope) -> impl Future<Output = io::Result<()>> + Send {
        self.events
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push("write");
        std::future::ready(Ok(()))
    }
}

/// Sink that rejects every write, modelling a lost peer.
struct FailingSink;

impl FrameSink<Envelope> for FailingSink {
    // The failure is immediate, so no `async` state machine is needed.
    fn send_frame(&mut self, _frame: Envelope) -> impl Future<Output = io::Result<()>> + Send {
        std::future::ready(Err(io::Error::other("peer lost")))
    }
}

/// Hooks recording the completion hook alongside a supplied terminator.
fn ordering_hooks(events: Arc<Mutex<Vec<&'static str>>>) -> ProtocolHooks<Envelope, ()> {
    ProtocolHooks {
        on_command_end: Some(Box::new(move |_ctx| {
            events
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .push("hook");
        })),
        stream_end: Some(Box::new(|_ctx| Some(Envelope::new(9, None, vec![])))),
        ..ProtocolHooks::default()
    }
}

fn recorded_events(events: &Arc<Mutex<Vec<&'static str>>>) -> Vec<&'static str> {
    events
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .clone()
}

/// Build a response that yields one frame and then drains.
async fn draining_response(is_streaming: bool) -> io::Result<Response<Envelope, ()>> {
    if is_streaming {
        let stream: FrameStream<Envelope, ()> = Box::pin(futures::stream::iter(vec![Ok(
            Envelope::new(1, None, vec![1]),
        )]));
        return Ok(Response::Stream(stream));
    }
    let (sender, receiver) = mpsc::channel(4);
    sender
        .send(Envelope::new(1, None, vec![1]))
        .await
        .map_err(|error| io::Error::other(format!("channel should accept a frame: {error}")))?;
    drop(sender);
    Ok(Response::MultiPacket(receiver))
}

/// Streams and drained channels both finish writing before the hook runs.
#[rstest]
#[case::stream(true)]
#[case::multi_packet(false)]
#[tokio::test]
async fn completion_hook_runs_after_the_terminator_is_written(#[case] is_streaming: bool) {
    let events = Arc::new(Mutex::new(Vec::new()));
    let mut actor = test_actor(ordering_hooks(Arc::clone(&events))).expect("actor should build");
    let mut sink = RecordingSink {
        events: Arc::clone(&events),
    };

    actor
        .install_response(
            draining_response(is_streaming)
                .await
                .expect("channel should accept a frame"),
            None,
            &mut sink,
        )
        .await
        .expect("response should drive to completion");

    assert_eq!(
        recorded_events(&events),
        vec!["write", "write", "hook"],
        "the completion hook must run only after the terminator write"
    );
}

#[tokio::test]
async fn completion_hook_is_skipped_when_the_terminator_write_fails() {
    let events = Arc::new(Mutex::new(Vec::new()));
    let mut actor = test_actor(ordering_hooks(Arc::clone(&events))).expect("actor should build");
    let stream: FrameStream<Envelope, ()> = Box::pin(futures::stream::empty());
    let mut sink = FailingSink;

    actor
        .install_response(Response::Stream(stream), None, &mut sink)
        .await
        .expect_err("a failed terminator write must be reported");

    assert!(
        recorded_events(&events).is_empty(),
        "a failed write must not report command completion"
    );
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
