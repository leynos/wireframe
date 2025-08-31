//! Error propagation and protocol hook tests for `ConnectionActor`.

use std::sync::{
    Arc,
    atomic::{AtomicUsize, Ordering},
};

use futures::stream;
use rstest::{fixture, rstest};
use serial_test::serial;
use tokio_util::sync::CancellationToken;
use wireframe::{
    ConnectionContext,
    ProtocolHooks,
    connection::ConnectionActor,
    push::PushQueues,
    response::WireframeError,
};
use wireframe_testing::{LoggerHandle, logger, push_expect};

#[fixture]
#[expect(
    unused_braces,
    reason = "rustc false positive for single line rstest fixtures"
)]
// allow(unfulfilled_lint_expectations): rustc occasionally fails to emit the expected
// lint for single-line rstest fixtures on stable.
#[allow(unfulfilled_lint_expectations)]
fn queues() -> (PushQueues<u8>, wireframe::push::PushHandle<u8>) {
    PushQueues::<u8>::builder()
        .high_capacity(8)
        .low_capacity(8)
        .build()
        .expect("failed to build PushQueues")
}

#[fixture]
#[expect(
    unused_braces,
    reason = "rustc false positive for single line rstest fixtures"
)]
// allow(unfulfilled_lint_expectations): rustc occasionally fails to emit the expected
// lint for single-line rstest fixtures on stable.
#[allow(unfulfilled_lint_expectations)]
fn shutdown_token() -> CancellationToken { CancellationToken::new() }

#[rstest]
#[tokio::test]
#[serial]
async fn before_send_hook_modifies_frames(
    queues: (PushQueues<u8>, wireframe::push::PushHandle<u8>),
    shutdown_token: CancellationToken,
) {
    let (queues, handle) = queues;
    push_expect!(handle.push_high_priority(1), "push high-priority");

    let stream = stream::iter(vec![Ok(2u8)]);
    let hooks = ProtocolHooks {
        before_send: Some(Box::new(|f: &mut u8, _ctx: &mut ConnectionContext| *f += 1)),
        ..ProtocolHooks::<u8, ()>::default()
    };

    let mut actor: ConnectionActor<_, ()> = ConnectionActor::with_hooks(
        queues,
        handle,
        Some(Box::pin(stream)),
        shutdown_token,
        hooks,
    );
    let mut out = Vec::new();
    actor.run(&mut out).await.expect("actor run failed");
    assert_eq!(out, vec![2, 3]);
}

#[rstest]
#[tokio::test]
#[serial]
async fn on_command_end_hook_runs(
    queues: (PushQueues<u8>, wireframe::push::PushHandle<u8>),
    shutdown_token: CancellationToken,
) {
    let (queues, handle) = queues;
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
        queues,
        handle,
        Some(Box::pin(stream)),
        shutdown_token,
        hooks,
    );
    let mut out = Vec::new();
    actor.run(&mut out).await.expect("actor run failed");
    assert_eq!(counter.load(Ordering::SeqCst), 1);
}

#[derive(Debug)]
enum TestError {
    Kaboom,
}

#[rstest]
#[tokio::test]
#[serial]
async fn error_propagation_from_stream(
    queues: (PushQueues<u8>, wireframe::push::PushHandle<u8>),
    shutdown_token: CancellationToken,
) {
    let (queues, handle) = queues;
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
        queues,
        handle,
        Some(Box::pin(stream)),
        shutdown_token,
        hooks,
    );
    let mut out = Vec::new();
    actor.run(&mut out).await.expect("actor run failed");
    assert_eq!(called.load(Ordering::SeqCst), 1);
    assert_eq!(out, vec![1, 2]);
}

#[rstest]
#[tokio::test]
#[serial]
async fn protocol_error_logs_warning(
    queues: (PushQueues<u8>, wireframe::push::PushHandle<u8>),
    shutdown_token: CancellationToken,
    mut logger: LoggerHandle,
) {
    let (queues, handle) = queues;
    let stream = stream::iter(vec![Err(WireframeError::Protocol(TestError::Kaboom))]);
    let mut actor: ConnectionActor<_, TestError> =
        ConnectionActor::new(queues, handle, Some(Box::pin(stream)), shutdown_token);
    let mut out = Vec::new();
    actor.run(&mut out).await.expect("actor run failed");
    assert!(out.is_empty());
    let mut found = false;
    while let Some(record) = logger.pop() {
        if record.level() == log::Level::Warn && record.args().contains("protocol error") {
            found = true;
            break;
        }
    }
    assert!(found, "warning log not found");
}

#[rstest]
#[tokio::test]
#[serial]
async fn io_error_terminates_connection(
    queues: (PushQueues<u8>, wireframe::push::PushHandle<u8>),
    shutdown_token: CancellationToken,
) {
    let (queues, handle) = queues;
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
}
