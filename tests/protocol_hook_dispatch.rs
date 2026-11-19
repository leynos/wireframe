//! Hook-dispatch boundary tests for a directly driven `ConnectionActor`.
//!
//! Every packet-oriented producer on the actor path — pushes, streamed
//! responses, multi-packet channel frames, stream-end terminators, and protocol
//! errors — must run `before_send` exactly once per emitted frame, in emission
//! order, through the connection's single hook owner. These tests assert the
//! whole ordered event sequence rather than a call count, so a duplicate
//! dispatch on any one path is visible. The prepared-application paths live in
//! `protocol_prepared_hook_dispatch`.
#![cfg(not(loom))]
#![forbid(unsafe_code)]

#[path = "common/dispatch_fixtures.rs"]
mod dispatch_fixtures;
#[path = "common/recording_protocol.rs"]
mod recording_protocol;
#[path = "common/recording_protocol_ownership.rs"]
mod recording_protocol_ownership;

use std::sync::{
    Arc,
    atomic::{AtomicUsize, Ordering},
};

use dispatch_fixtures::{actor_with_hooks, queues, run_error};
use futures::stream;
use recording_protocol::{HookEvent, MARKER, RecordingProtocolFixture};
use recording_protocol_ownership::{drop_count, is_alive, shared, strong_count, weak_handle};
use rstest::rstest;
use tokio::sync::mpsc;
use wireframe::{
    codec::EofError,
    connection::FairnessConfig,
    hooks::{ConnectionContext, CustomProtocolHooks, ProtocolHooks},
    response::WireframeError,
};
use wireframe_testing::TestResult;

/// Frame returned by the recording protocol's `stream_end_frame`.
const TERMINATOR: u8 = 0xee;

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
        ProtocolHooks::from_protocol(&shared(&fixture)),
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
            .map_err(|error| std::io::Error::other(format!("send multi-packet frame: {error}")))?;
    }
    drop(tx);

    let mut actor = actor_with_hooks(
        queues,
        handle,
        None,
        ProtocolHooks::from_protocol(&shared(&fixture)),
    );
    actor
        .set_multi_packet(Some(rx))
        .map_err(|error| std::io::Error::other(format!("set multi-packet: {error}")))?;
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
        ProtocolHooks::from_protocol(&shared(&fixture)),
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
    let protocol = shared(&fixture);
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
        drop_count(&fixture),
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

/// The actor path dispatches the explicit EOF hook itself, including the
/// partial bytes it received.
///
/// This covers a caller that drives a `ConnectionActor` and reports
/// end-of-file to it directly. It is not evidence of runtime EOF dispatch on
/// the prepared path; `prepared_connection_dispatches_setup_once_and_no_eof_hook`
/// pins that contract instead.
#[rstest]
fn explicit_eof_records_clean_and_partial_events() {
    let fixture = RecordingProtocolFixture::<Vec<u8>>::new(None);
    let mut hooks = ProtocolHooks::from_protocol(&shared(&fixture));
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
