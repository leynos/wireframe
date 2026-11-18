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

#[path = "common/protocol_request.rs"]
mod protocol_request;

#[path = "common/recording_protocol.rs"]
mod recording_protocol;

#[path = "common/tagging_codec.rs"]
mod tagging_codec;

use std::{
    io,
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    },
};

use bytes::Bytes;
use futures::stream;
use protocol_request::request_frame;
use recording_protocol::{
    HookEvent,
    MARKER,
    RecordingProtocolFixture,
    is_alive,
    strong_count,
    weak_handle,
};
use rstest::rstest;
use tagging_codec::{TaggingCodec, WRAP_TAG};
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;
use wireframe::{
    app::{Envelope, Handler, ResponseHandler},
    codec::EofError,
    connection::{ConnectionActor, ConnectionChannels, FairnessConfig},
    hooks::{ConnectionContext, CustomProtocolHooks, ProtocolHooks},
    push::{PushHandle, PushQueues},
    response::{FrameStream, Response, WireframeError},
    serializer::BincodeSerializer,
};
use wireframe_testing::{TestResult, decode_frames, prepare_and_drive_with_frames};

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
fn assert_no_eof_event<F: std::fmt::Debug>(events: &[HookEvent<F>]) {
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
    let mut context = ConnectionContext::default();

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
    let mut context = ConnectionContext::default();

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
    let mut context = ConnectionContext::default();

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
                error: EofError::CleanClose,
                partial: Vec::new(),
            },
            HookEvent::Eof {
                error: EofError::MidFrame {
                    bytes_received: 3,
                    expected: 8,
                },
                partial: vec![1, 2, 3],
            },
            HookEvent::Eof {
                error: EofError::MidHeader {
                    bytes_received: 1,
                    header_size: 4,
                },
                partial: vec![9],
            },
        ],
        "EOF dispatch forwards the exact error and partial bytes it saw"
    );
}

/// Handler that accepts an envelope without changing it.
fn echo_handler() -> Handler<Envelope> { Arc::new(|_env: &Envelope| Box::pin(async {})) }

/// Handler that streams two frames before completing.
fn two_frame_stream_handler() -> ResponseHandler<Envelope> {
    Arc::new(|_request: &Envelope| {
        Box::pin(async {
            let frames: FrameStream<Envelope, ()> = Box::pin(stream::iter(vec![
                Ok(Envelope::new(2, None, vec![1_u8])),
                Ok(Envelope::new(2, None, vec![2_u8])),
            ]));
            Response::Stream(frames)
        })
    })
}

/// The hook observes the codec-wrapped frame, after the serializer and the
/// codec have run and immediately before the transport write.
///
/// That is the boundary ADR 012's 2026-11-18 addendum records: the hook takes
/// `&mut F::Frame`, so it cannot see the `Envelope` before serialization, and
/// moving it there would reopen ADR 010.
#[rstest]
#[tokio::test]
async fn prepared_response_runs_before_send_once_after_serialization() -> TestResult<()> {
    let fixture = RecordingProtocolFixture::<Bytes>::new(None);
    let app = ResponseApp::new()?
        .route(1, echo_handler())?
        .with_codec(TaggingCodec::new(1024))
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

    assert_eq!(
        observed.last(),
        Some(&WRAP_TAG),
        "the hook must run after the codec wrapped the payload: {observed:?}"
    );
    let mut expected = observed.to_vec();
    expected.push(MARKER);
    assert_eq!(
        decoded, &expected,
        "the hook must observe the codec-wrapped frame that the peer decodes"
    );
    Ok(())
}

/// A prepared stream runs `before_send` once per frame and once on the
/// terminator, in emission order.
#[rstest]
#[tokio::test]
async fn prepared_stream_runs_before_send_once_per_frame_and_terminator() -> TestResult<()> {
    const TERMINATOR_BYTES: &[u8] = b"END";
    let fixture =
        RecordingProtocolFixture::<Bytes>::new(Some(Bytes::from_static(TERMINATOR_BYTES)));
    let app = ResponseApp::new()?
        .route_response(2, two_frame_stream_handler())?
        .with_protocol(fixture.protocol());

    let bytes = prepare_and_drive_with_frames(app, vec![request_frame(2)?]).await?;

    let events = fixture.events();
    let [
        HookEvent::BeforeSend(first),
        HookEvent::BeforeSend(second),
        HookEvent::StreamEnd,
        HookEvent::BeforeSend(terminator),
        HookEvent::CommandEnd,
    ] = events.as_slice()
    else {
        return Err(format!(
            "expected two hooked frames, a hooked terminator, then completion, got {events:?}"
        )
        .into());
    };
    assert_eq!(
        &terminator[..],
        TERMINATOR_BYTES,
        "the terminator reaches the hook as the protocol supplied it"
    );
    let expected: Vec<Vec<u8>> = [first, second, terminator]
        .into_iter()
        .map(|frame| {
            let mut marked = frame.to_vec();
            marked.push(MARKER);
            marked
        })
        .collect();
    assert_eq!(
        decode_frames(&bytes)?,
        expected,
        "each streamed frame and the terminator are hooked exactly once"
    );
    Ok(())
}

/// A prepared connection dispatches `on_connection_setup` exactly once and
/// never `on_eof`, including when the peer closes mid-frame.
///
/// Setup runs at the connection boundary, because that is where the push
/// handle and the registry entry come into existence; the reference used for
/// that call is not retained, so the connection still holds exactly one strong
/// protocol reference. The end-of-file callback is outside the prepared
/// contract: a mid-frame close is reported to the caller as an I/O error.
/// ADR 012's 2026-11-20 addendum records that split.
#[rstest]
#[tokio::test]
async fn prepared_connection_dispatches_setup_once_and_no_eof_hook() -> TestResult<()> {
    let fixture = RecordingProtocolFixture::<Bytes>::new(None);
    let app = ResponseApp::new()?.with_protocol(fixture.protocol());

    // One byte is a truncated length header, so the peer's close is a mid-frame
    // end-of-file while the protocol is installed. The runtime reports it to the
    // caller as an I/O error; the protocol must not be told about it.
    let outcome = prepare_and_drive_with_frames(app, vec![vec![0x00]]).await;
    assert!(
        outcome.is_err(),
        "a truncated length header must reach the caller: {outcome:?}"
    );

    let events = fixture.events();
    assert_eq!(
        events,
        vec![HookEvent::Setup],
        "a prepared connection dispatches setup once and never EOF: {events:?}"
    );
    Ok(())
}
