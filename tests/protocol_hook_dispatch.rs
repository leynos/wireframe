//! Hook-dispatch boundary tests for packet-oriented outbound frames.
//!
//! Every packet-oriented producer — server pushes, streamed responses,
//! multi-packet channel frames, stream-end terminators, protocol errors, and
//! application handler responses — must run `before_send` exactly once per
//! emitted frame, in emission order, through the connection's single hook
//! owner. These tests assert the whole ordered event sequence rather than a
//! call count, so a duplicate dispatch on any one path is visible.
#![cfg(not(loom))]
#![forbid(unsafe_code)]

#[path = "common/recording_protocol.rs"]
mod recording_protocol;

use std::{
    io,
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    },
};

use bytes::Bytes;
use futures::stream;
use recording_protocol::{
    HookEvent,
    MARKER,
    RecordingProtocolFixture,
    is_alive,
    strong_count,
    weak_handle,
};
use rstest::rstest;
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;
use wireframe::{
    app::{Envelope, Handler},
    codec::EofError,
    connection::{ConnectionActor, ConnectionChannels, FairnessConfig},
    hooks::{ConnectionContext, CustomProtocolHooks, ProtocolHooks},
    push::{PushHandle, PushQueues},
    response::{FrameStream, WireframeError},
    serializer::{BincodeSerializer, Serializer},
};
use wireframe_testing::{TestResult, decode_frames, encode_frame, prepare_and_drive_with_frames};

/// Frame returned by the recording protocol's `stream_end_frame`.
const TERMINATOR: u8 = 0xee;

/// Application type using the default length-delimited codec (`Bytes` frames).
type ResponseApp = wireframe::app::WireframeApp<BincodeSerializer, (), Envelope>;

/// Push queues paired with the handle a test uses to prefill them.
type TestQueues = (PushQueues<Vec<u8>>, PushHandle<Vec<u8>>);

/// Build push queues sized for the frames each test prefills.
fn queues() -> TestResult<TestQueues> {
    Ok(PushQueues::<Vec<u8>>::builder()
        .high_capacity(8)
        .low_capacity(8)
        .unlimited()
        .build()?)
}

/// Build an actor over prefilled queues and the supplied hooks.
fn actor_with_hooks(
    queues: PushQueues<Vec<u8>>,
    handle: PushHandle<Vec<u8>>,
    response: Option<FrameStream<Vec<u8>, ()>>,
    hooks: ProtocolHooks<Vec<u8>, ()>,
) -> ConnectionActor<Vec<u8>, ()> {
    ConnectionActor::with_hooks(
        ConnectionChannels::new(queues, handle),
        response,
        CancellationToken::new(),
        hooks,
    )
}

/// Present an actor error as a test failure without losing its debug form.
fn run_error(error: &WireframeError<()>) -> io::Error {
    io::Error::other(format!("actor run failed: {error:?}"))
}

/// Only one high-priority frame may precede the low-priority queue.
fn alternating_fairness() -> FairnessConfig {
    FairnessConfig {
        max_high_before_low: 1,
        time_slice: None,
    }
}

/// Assert no end-of-file event was produced automatically.
fn assert_no_eof_event(events: &[HookEvent<Vec<u8>>]) {
    assert!(
        !events
            .iter()
            .any(|event| matches!(event, HookEvent::Eof { .. })),
        "the connection loop must not dispatch on_eof automatically: {events:?}"
    );
}

#[rstest]
#[tokio::test]
async fn push_and_streamed_frames_run_before_send_once() -> TestResult<()> {
    let fixture = RecordingProtocolFixture::<Vec<u8>>::new(Some(vec![TERMINATOR]));
    let (queues, handle) = queues()?;
    handle.push_high_priority(vec![1]).await?;
    handle.push_low_priority(vec![2]).await?;
    let response = stream::iter(vec![Ok(vec![3_u8])]);
    let mut actor = actor_with_hooks(
        queues,
        handle,
        Some(Box::pin(response)),
        ProtocolHooks::from_protocol(&fixture.shared()),
    );
    actor.set_fairness(alternating_fairness());

    let mut out = Vec::new();
    actor
        .run(&mut out)
        .await
        .map_err(|error| run_error(&error))?;

    assert_eq!(
        out,
        vec![
            vec![1, MARKER],
            vec![2, MARKER],
            vec![3, MARKER],
            vec![TERMINATOR, MARKER],
        ],
        "each emitted frame must carry exactly one protocol marker"
    );
    let events = fixture.events();
    assert_eq!(
        events,
        vec![
            HookEvent::Setup,
            HookEvent::BeforeSend(vec![1]),
            HookEvent::BeforeSend(vec![2]),
            HookEvent::BeforeSend(vec![3]),
            HookEvent::StreamEnd,
            HookEvent::BeforeSend(vec![TERMINATOR]),
            HookEvent::CommandEnd,
        ],
        "hook order must match emission order with no duplicate dispatch"
    );
    assert_no_eof_event(&events);
    Ok(())
}

#[rstest]
#[tokio::test]
async fn multi_packet_frames_and_terminator_run_before_send_once() -> TestResult<()> {
    let fixture = RecordingProtocolFixture::<Vec<u8>>::new(Some(vec![TERMINATOR]));
    let (queues, handle) = queues()?;
    let (tx, rx) = mpsc::channel(4);
    for frame in [vec![7_u8], vec![8]] {
        tx.send(frame)
            .await
            .map_err(|error| io::Error::other(format!("send multi-packet frame: {error}")))?;
    }
    drop(tx);

    let mut actor = actor_with_hooks(
        queues,
        handle,
        None,
        ProtocolHooks::from_protocol(&fixture.shared()),
    );
    actor
        .set_multi_packet(Some(rx))
        .map_err(|error| io::Error::other(format!("set multi-packet: {error}")))?;
    actor.set_fairness(alternating_fairness());

    let mut out = Vec::new();
    actor
        .run(&mut out)
        .await
        .map_err(|error| run_error(&error))?;

    assert_eq!(
        out,
        vec![vec![7, MARKER], vec![8, MARKER], vec![TERMINATOR, MARKER],],
        "multi-packet frames and their terminator are hooked once each"
    );
    assert_eq!(
        fixture.events(),
        vec![
            HookEvent::Setup,
            HookEvent::BeforeSend(vec![7]),
            HookEvent::BeforeSend(vec![8]),
            HookEvent::StreamEnd,
            HookEvent::BeforeSend(vec![TERMINATOR]),
            HookEvent::CommandEnd,
        ],
        "multi-packet hook order must match emission order"
    );
    Ok(())
}

#[rstest]
#[tokio::test]
async fn protocol_error_runs_handle_error_then_command_end() -> TestResult<()> {
    let fixture = RecordingProtocolFixture::<Vec<u8>>::new(None);
    let (queues, handle) = queues()?;
    let response = stream::iter(vec![Err::<Vec<u8>, WireframeError<()>>(
        WireframeError::Protocol(()),
    )]);
    let mut actor = actor_with_hooks(
        queues,
        handle,
        Some(Box::pin(response)),
        ProtocolHooks::from_protocol(&fixture.shared()),
    );

    let mut out = Vec::new();
    actor
        .run(&mut out)
        .await
        .map_err(|error| run_error(&error))?;

    assert!(out.is_empty(), "a protocol error emits no frame: {out:?}");
    assert_eq!(
        fixture.events(),
        vec![
            HookEvent::Setup,
            HookEvent::ProtocolError,
            HookEvent::CommandEnd,
        ],
        "a protocol error runs the error hook once, then command end once"
    );
    Ok(())
}

#[rstest]
#[tokio::test]
async fn repeated_explicit_setup_runs_once_per_hooks_value() -> TestResult<()> {
    let fixture = RecordingProtocolFixture::<Vec<u8>>::new(None);
    let protocol = fixture.shared();
    let weak = weak_handle(&protocol);
    let mut hooks = ProtocolHooks::from_protocol(&protocol);
    assert_eq!(
        strong_count(&protocol),
        2,
        "the hooks value owns exactly one reference"
    );
    let (_queues, handle) = queues()?;
    let mut context = ConnectionContext;

    hooks.on_connection_setup(handle.clone(), &mut context);
    hooks.on_connection_setup(handle, &mut context);

    assert_eq!(
        fixture.events(),
        vec![HookEvent::Setup],
        "a protocol-backed hooks value forwards setup at most once"
    );

    drop(hooks);
    assert_eq!(
        strong_count(&protocol),
        1,
        "dropping the hooks releases its single reference"
    );
    drop(protocol);
    assert!(!is_alive(&weak), "no strong reference remains");
    assert_eq!(
        fixture.drops(),
        1,
        "exactly one installed protocol instance was dropped"
    );
    Ok(())
}

#[rstest]
#[tokio::test]
async fn custom_setup_callback_runs_once_per_hooks_value() -> TestResult<()> {
    let calls = Arc::new(AtomicUsize::new(0));
    let counter = Arc::clone(&calls);
    let mut hooks: ProtocolHooks<Vec<u8>, ()> = CustomProtocolHooks {
        on_connection_setup: Some(Box::new(move |_handle, _context| {
            counter.fetch_add(1, Ordering::SeqCst);
        })),
        ..CustomProtocolHooks::<Vec<u8>, ()>::default()
    }
    .into();
    let (_queues, handle) = queues()?;
    let mut context = ConnectionContext;

    hooks.on_connection_setup(handle.clone(), &mut context);
    hooks.on_connection_setup(handle, &mut context);

    assert_eq!(
        calls.load(Ordering::SeqCst),
        1,
        "a custom setup callback is consumed by its first call"
    );
    Ok(())
}

#[rstest]
fn explicit_eof_records_clean_and_partial_events() {
    let fixture = RecordingProtocolFixture::<Vec<u8>>::new(None);
    let mut hooks = ProtocolHooks::from_protocol(&fixture.shared());
    let mut context = ConnectionContext;

    hooks.on_eof(&EofError::CleanClose, &[], &mut context);
    hooks.on_eof(
        &EofError::MidFrame {
            bytes_received: 3,
            expected: 8,
        },
        &[1, 2, 3],
        &mut context,
    );
    hooks.on_eof(
        &EofError::MidHeader {
            bytes_received: 1,
            header_size: 4,
        },
        &[9],
        &mut context,
    );

    assert_eq!(
        fixture.events(),
        vec![
            HookEvent::Eof {
                clean: true,
                partial_len: 0,
            },
            HookEvent::Eof {
                clean: false,
                partial_len: 3,
            },
            HookEvent::Eof {
                clean: false,
                partial_len: 1,
            },
        ],
        "EOF dispatch reports the close kind and the partial bytes it saw"
    );
}

/// Encode one request envelope into a length-delimited frame.
fn request_frame(id: u32) -> TestResult<Vec<u8>> {
    let payload = BincodeSerializer.serialize(&Envelope::new(id, Some(7), Vec::new()))?;
    let mut codec = tokio_util::codec::LengthDelimitedCodec::builder()
        .max_frame_length(1024)
        .new_codec();
    Ok(encode_frame(&mut codec, payload)?)
}

/// Handler that accepts an envelope without changing it.
fn echo_handler() -> Handler<Envelope> { Arc::new(|_env: &Envelope| Box::pin(async {})) }

#[rstest]
#[tokio::test]
async fn prepared_response_runs_before_send_once_on_the_wrapped_frame() -> TestResult<()> {
    let fixture = RecordingProtocolFixture::<Bytes>::new(None);
    let app = ResponseApp::new()?
        .route(1, echo_handler())?
        .with_protocol(fixture.protocol());

    let bytes = prepare_and_drive_with_frames(app, vec![request_frame(1)?]).await?;

    let frames = decode_frames(&bytes)?;
    let [decoded] = frames.as_slice() else {
        return Err(format!("expected exactly one response frame, got {}", frames.len()).into());
    };
    let events = fixture.events();
    let [HookEvent::BeforeSend(observed), HookEvent::CommandEnd] = events.as_slice() else {
        return Err(format!(
            "expected before_send then command end on the prepared path, got {events:?}"
        )
        .into());
    };

    let mut expected = observed.to_vec();
    expected.push(MARKER);
    assert_eq!(
        decoded, &expected,
        "the hook must observe the codec-wrapped frame that the peer decodes"
    );
    Ok(())
}
