//! Unit tests for connection actor scheduling and response installation.

mod completion;

mod grace;

mod grace_warning;

mod push_ordering;

mod shutdown_drain;

mod stream_correlation;

mod vec_delivery;

mod write_bound_property;

mod writes;

use std::sync::{
    Arc,
    atomic::{AtomicUsize, Ordering},
};

use rstest::{fixture, rstest};
use tokio::{
    sync::mpsc,
    time::{Duration, timeout},
};
use tokio_util::sync::CancellationToken;

use super::{
    ConnectionActor,
    ConnectionChannels,
    UntrackedActorInit,
    event::Event,
    multi_packet::MultiPacketContext,
    output::ActiveOutput,
    state::ActorState,
};
use crate::{
    NoProtocolError,
    app::Envelope,
    correlation::CorrelatableFrame,
    hooks::ProtocolHooks,
    push::{PushConfigError, PushQueues},
    response::{FrameStream, Response, WireframeError},
};

fn test_actor(
    hooks: ProtocolHooks<Envelope, ()>,
) -> Result<ConnectionActor<Envelope, ()>, crate::push::PushConfigError> {
    test_actor_with_shutdown(CancellationToken::new(), hooks)
}

/// Build an actor whose shutdown token the test controls.
fn test_actor_with_shutdown(
    shutdown: CancellationToken,
    hooks: ProtocolHooks<Envelope, ()>,
) -> Result<ConnectionActor<Envelope, ()>, crate::push::PushConfigError> {
    let (queues, handle) = PushQueues::<Envelope>::builder()
        .high_capacity(1)
        .low_capacity(1)
        .build()?;
    Ok(ConnectionActor::new_untracked(UntrackedActorInit {
        channels: ConnectionChannels::new(queues, handle),
        response: None,
        shutdown,
        hooks,
        ctx: crate::hooks::ConnectionContext::default(),
    }))
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

/// Active-output configurations exercised by the correlation-stamping matrix.
enum StampingSetup {
    /// No active output source is installed.
    NoOutput,
    /// A multi-packet context exists but stamping is disabled.
    DisabledContext,
    /// A multi-packet channel stamps the supplied identifier.
    EnabledWithCorrelation(u64),
    /// A multi-packet channel is active but no identifier was supplied.
    EnabledWithoutCorrelation,
}

/// Build an actor with rate-unlimited push queues for correlation tests.
#[fixture]
fn correlation_actor() -> Result<ConnectionActor<Envelope, NoProtocolError>, PushConfigError> {
    let (queues, handle) = PushQueues::<Envelope>::builder()
        .high_capacity(1)
        .low_capacity(1)
        .unlimited()
        .build()?;
    Ok(ConnectionActor::new(
        queues,
        handle,
        None,
        CancellationToken::new(),
    ))
}

/// Correlation stamping replaces a frame's identifier only when an enabled
/// context has an identifier to stamp; every other output state preserves the
/// identifier the producer supplied.
#[rstest]
#[case::no_active_output_preserves_existing_id(StampingSetup::NoOutput, Some(99))]
#[case::disabled_context_preserves_existing_id(StampingSetup::DisabledContext, Some(99))]
#[case::enabled_stamps_configured_id(StampingSetup::EnabledWithCorrelation(7), Some(7))]
#[case::enabled_without_id_preserves_existing_id(
    StampingSetup::EnabledWithoutCorrelation,
    Some(99)
)]
fn multi_packet_correlation_stamping_matches_the_active_output(
    #[case] setup: StampingSetup,
    #[case] expected: Option<u64>,
    correlation_actor: Result<ConnectionActor<Envelope, NoProtocolError>, PushConfigError>,
) -> Result<(), Box<dyn std::error::Error>> {
    let mut actor = correlation_actor?;
    // A bounded channel's receiver stays open only while its sender lives, so
    // hold every sender until after the helper call under test.
    let mut keep_alive = Vec::new();
    match setup {
        StampingSetup::NoOutput => {}
        StampingSetup::DisabledContext => {
            // Through the actor API `is_stamping_enabled` is only ever true,
            // so install a disabled context directly. This pins the helper's
            // early return; the predicate's own both-polarity coverage lives
            // in `multi_packet::tests`.
            actor.active_output = ActiveOutput::MultiPacket(MultiPacketContext::new());
        }
        StampingSetup::EnabledWithCorrelation(correlation_id) => {
            let (tx, rx) = mpsc::channel(1);
            keep_alive.push(tx);
            actor.set_multi_packet_with_correlation(Some(rx), Some(correlation_id))?;
        }
        StampingSetup::EnabledWithoutCorrelation => {
            let (tx, rx) = mpsc::channel(1);
            keep_alive.push(tx);
            actor.set_multi_packet(Some(rx))?;
        }
    }

    let mut frame = Envelope::new(1, Some(99), vec![42]);
    actor.apply_multi_packet_correlation(&mut frame);

    let actual = frame.correlation_id();
    if actual != expected {
        return Err(
            format!("frame correlation mismatch: expected {expected:?}, got {actual:?}").into(),
        );
    }
    Ok(())
}
