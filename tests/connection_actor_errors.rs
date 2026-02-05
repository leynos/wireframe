#![cfg(not(loom))]
//! Error propagation and protocol hook tests for `ConnectionActor`.

use std::{
    io,
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    },
};

use futures::stream;
use rstest::{fixture, rstest};
use serial_test::serial;
use tokio_util::sync::CancellationToken;
use wireframe::{
    ConnectionContext,
    ProtocolHooks,
    connection::{ConnectionActor, ConnectionChannels},
    push::PushQueues,
    response::WireframeError,
};
use wireframe_testing::{LoggerHandle, TestResult, logger, push_expect};

#[fixture]
fn queues()
-> Result<(PushQueues<u8>, wireframe::push::PushHandle<u8>), wireframe::push::PushConfigError> {
    // Push queues with default capacities for error propagation tests
    PushQueues::<u8>::builder()
        .high_capacity(8)
        .low_capacity(8)
        .build()
}

#[fixture]
fn shutdown_token() -> CancellationToken {
    // Shutdown token for connection actor tests
    CancellationToken::new()
}

#[rstest]
#[tokio::test]
#[serial]
async fn before_send_hook_modifies_frames(
    queues: Result<
        (PushQueues<u8>, wireframe::push::PushHandle<u8>),
        wireframe::push::PushConfigError,
    >,
    shutdown_token: CancellationToken,
) -> TestResult {
    let (queues, handle) = queues?;
    push_expect!(handle.push_high_priority(1), "push high-priority");

    let stream = stream::iter(vec![Ok(2u8)]);
    let hooks = ProtocolHooks {
        before_send: Some(Box::new(|f: &mut u8, _ctx: &mut ConnectionContext| *f += 1)),
        ..ProtocolHooks::<u8, ()>::default()
    };

    let mut actor: ConnectionActor<_, ()> = ConnectionActor::with_hooks(
        ConnectionChannels::new(queues, handle),
        Some(Box::pin(stream)),
        shutdown_token,
        hooks,
    );
    let mut out = Vec::new();
    actor
        .run(&mut out)
        .await
        .map_err(|e| io::Error::other(format!("actor run failed: {e:?}")))?;
    assert_eq!(out, vec![2, 3]);
    Ok(())
}

#[rstest]
#[tokio::test]
#[serial]
async fn on_command_end_hook_runs(
    queues: Result<
        (PushQueues<u8>, wireframe::push::PushHandle<u8>),
        wireframe::push::PushConfigError,
    >,
    shutdown_token: CancellationToken,
) -> TestResult {
    let (queues, handle) = queues?;
    let stream = stream::iter(vec![Ok(1u8)]);

    let counter = Arc::new(AtomicUsize::new(0));
    let c = counter.clone();
    let hooks = ProtocolHooks {
        on_command_end: Some(Box::new(move |_ctx: &mut ConnectionContext| {
            c.fetch_add(1, Ordering::SeqCst);
        })),
        ..ProtocolHooks::<u8, ()>::default()
    };

    let mut actor: ConnectionActor<_, ()> = ConnectionActor::with_hooks(
        ConnectionChannels::new(queues, handle),
        Some(Box::pin(stream)),
        shutdown_token,
        hooks,
    );
    let mut out = Vec::new();
    actor
        .run(&mut out)
        .await
        .map_err(|e| io::Error::other(format!("actor run failed: {e:?}")))?;
    assert_eq!(counter.load(Ordering::SeqCst), 1);
    Ok(())
}

#[derive(Debug)]
enum TestError {
    Kaboom,
}

#[rstest]
#[tokio::test]
#[serial]
async fn error_propagation_from_stream(
    queues: Result<
        (PushQueues<u8>, wireframe::push::PushHandle<u8>),
        wireframe::push::PushConfigError,
    >,
    shutdown_token: CancellationToken,
) -> TestResult {
    let (queues, handle) = queues?;
    let stream = stream::iter(vec![
        Ok(1u8),
        Ok(2u8),
        Err(WireframeError::Protocol(TestError::Kaboom)),
    ]);
    let called = Arc::new(AtomicUsize::new(0));
    let c = called.clone();
    let hooks = ProtocolHooks {
        handle_error: Some(Box::new(
            move |_e: TestError, _ctx: &mut ConnectionContext| {
                c.fetch_add(1, Ordering::SeqCst);
            },
        )),
        ..ProtocolHooks::<u8, TestError>::default()
    };
    let mut actor: ConnectionActor<_, TestError> = ConnectionActor::with_hooks(
        ConnectionChannels::new(queues, handle),
        Some(Box::pin(stream)),
        shutdown_token,
        hooks,
    );
    let mut out = Vec::new();
    actor
        .run(&mut out)
        .await
        .map_err(|e| io::Error::other(format!("actor run failed: {e:?}")))?;
    assert_eq!(called.load(Ordering::SeqCst), 1);
    assert_eq!(out, vec![1, 2]);
    Ok(())
}

#[rstest]
#[tokio::test]
#[serial]
async fn protocol_error_logs_warning(
    queues: Result<
        (PushQueues<u8>, wireframe::push::PushHandle<u8>),
        wireframe::push::PushConfigError,
    >,
    shutdown_token: CancellationToken,
    mut logger: LoggerHandle,
) -> TestResult {
    let (queues, handle) = queues?;
    let stream = stream::iter(vec![Err(WireframeError::Protocol(TestError::Kaboom))]);
    let mut actor: ConnectionActor<_, TestError> =
        ConnectionActor::new(queues, handle, Some(Box::pin(stream)), shutdown_token);
    let mut out = Vec::new();
    actor
        .run(&mut out)
        .await
        .map_err(|e| io::Error::other(format!("actor run failed: {e:?}")))?;
    assert!(out.is_empty());
    let mut found = false;
    while let Some(record) = logger.pop() {
        if record.level() == log::Level::Warn && record.args().contains("protocol error") {
            found = true;
            break;
        }
    }
    assert!(found, "warning log not found");
    Ok(())
}

#[rstest]
#[tokio::test]
#[serial]
async fn io_error_terminates_connection(
    queues: Result<
        (PushQueues<u8>, wireframe::push::PushHandle<u8>),
        wireframe::push::PushConfigError,
    >,
    shutdown_token: CancellationToken,
) -> TestResult {
    let (queues, handle) = queues?;
    let stream = stream::iter(vec![
        Ok(1u8),
        Err(WireframeError::Io(std::io::Error::other("fail"))),
    ]);
    let mut actor: ConnectionActor<_, ()> =
        ConnectionActor::new(queues, handle, Some(Box::pin(stream)), shutdown_token);
    let mut out = Vec::new();
    let result = actor.run(&mut out).await;
    assert!(matches!(result, Err(WireframeError::Io(_))));
    assert_eq!(out, vec![1]);
    Ok(())
}
