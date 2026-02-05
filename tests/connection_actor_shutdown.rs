#![cfg(not(loom))]
//! Shutdown and back-pressure tests for `ConnectionActor`.

use futures::stream;
use rstest::{fixture, rstest};
use serial_test::serial;
use tokio::time::{Duration, sleep, timeout};
use tokio_util::{sync::CancellationToken, task::TaskTracker};
use wireframe::{connection::ConnectionActor, push::PushQueues};
use wireframe_testing::{TestResult, push_expect};

#[fixture]
fn queues()
-> Result<(PushQueues<u8>, wireframe::push::PushHandle<u8>), wireframe::push::PushConfigError> {
    // Push queues with default capacities for shutdown tests
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
async fn shutdown_signal_precedence(
    queues: Result<
        (PushQueues<u8>, wireframe::push::PushHandle<u8>),
        wireframe::push::PushConfigError,
    >,
    shutdown_token: CancellationToken,
) {
    let (queues, handle) = queues.expect("fixture should build queues");
    shutdown_token.cancel();
    let mut actor: ConnectionActor<_, ()> =
        ConnectionActor::new(queues, handle, None, shutdown_token);
    // drop the handle after actor creation to mimic early disconnection
    let mut out = Vec::new();
    actor.run(&mut out).await.expect("actor run failed");
    assert!(out.is_empty());
}

#[rstest]
#[tokio::test]
#[serial]
async fn complete_draining_of_sources(
    queues: Result<
        (PushQueues<u8>, wireframe::push::PushHandle<u8>),
        wireframe::push::PushConfigError,
    >,
    shutdown_token: CancellationToken,
) {
    let (queues, handle) = queues.expect("fixture should build queues");
    push_expect!(handle.push_high_priority(1), "push high-priority");

    let stream = stream::iter(vec![Ok(2u8), Ok(3u8)]);
    let mut actor: ConnectionActor<_, ()> =
        ConnectionActor::new(queues, handle, Some(Box::pin(stream)), shutdown_token);
    // drop handle after actor setup
    let mut out = Vec::new();
    actor.run(&mut out).await.expect("actor run failed");
    assert_eq!(out, vec![1, 2, 3]);
}

#[rstest]
#[tokio::test]
#[serial]
async fn interleaved_shutdown_during_stream(
    queues: Result<
        (PushQueues<u8>, wireframe::push::PushHandle<u8>),
        wireframe::push::PushConfigError,
    >,
    shutdown_token: CancellationToken,
) {
    let (queues, handle) = queues.expect("fixture should build queues");
    let token = shutdown_token.clone();
    tokio::spawn(async move {
        sleep(Duration::from_millis(50)).await;
        token.cancel();
    });

    let stream = stream::unfold(1u8, |i| async move {
        if i <= 5 {
            sleep(Duration::from_millis(20)).await;
            Some((Ok(i), i + 1))
        } else {
            None
        }
    });
    let mut actor: ConnectionActor<_, ()> =
        ConnectionActor::new(queues, handle, Some(Box::pin(stream)), shutdown_token);
    let mut out = Vec::new();
    actor.run(&mut out).await.expect("actor run failed");
    assert!(!out.is_empty() && out.len() < 5);
}

#[rstest]
#[tokio::test]
#[serial]
async fn push_queue_exhaustion_backpressure() {
    let (mut queues, handle) = PushQueues::<u8>::builder()
        .high_capacity(1)
        .low_capacity(1)
        .build()
        .expect("failed to build PushQueues");
    push_expect!(handle.push_high_priority(1), "push high-priority");

    let blocked = timeout(Duration::from_millis(200), handle.push_high_priority(2)).await;
    assert!(blocked.is_err());

    // clean up without exposing internal fields
    queues.close();
}

#[rstest]
#[tokio::test]
#[serial]
async fn graceful_shutdown_waits_for_tasks() -> TestResult {
    let tracker = TaskTracker::new();
    let token = CancellationToken::new();

    let mut handles = Vec::new();
    for _ in 0..5 {
        let (queues, handle) = PushQueues::<u8>::builder()
            .high_capacity(1)
            .low_capacity(1)
            .build()?;
        let mut actor: ConnectionActor<_, ()> =
            ConnectionActor::new(queues, handle.clone(), None, token.clone());
        handles.push(handle);
        tracker.spawn(async move {
            let mut out = Vec::new();
            let _ = actor.run(&mut out).await;
        });
    }

    token.cancel();
    tracker.close();

    assert!(
        timeout(Duration::from_millis(500), tracker.wait())
            .await
            .is_ok(),
    );
    Ok(())
}

#[rstest]
#[tokio::test]
#[serial]
async fn connection_count_decrements_on_abort(
    queues: Result<
        (PushQueues<u8>, wireframe::push::PushHandle<u8>),
        wireframe::push::PushConfigError,
    >,
) {
    let (queues, handle) = queues.expect("fixture should build queues");
    let token = CancellationToken::new();
    token.cancel();

    let before = wireframe::connection::active_connection_count();
    let mut actor: ConnectionActor<_, ()> = ConnectionActor::new(queues, handle, None, token);
    let during = wireframe::connection::active_connection_count();
    assert_eq!(during, before + 1);

    let mut out = Vec::new();
    actor.run(&mut out).await.expect("actor run failed");
    let after = wireframe::connection::active_connection_count();
    assert_eq!(during - after, 1);
}

#[rstest]
#[tokio::test]
#[serial]
async fn connection_count_decrements_on_close(
    queues: Result<
        (PushQueues<u8>, wireframe::push::PushHandle<u8>),
        wireframe::push::PushConfigError,
    >,
    shutdown_token: CancellationToken,
) {
    let (queues, handle) = queues.expect("fixture should build queues");
    let before = wireframe::connection::active_connection_count();
    let stream = stream::iter(vec![Ok(1u8)]);
    let mut actor: ConnectionActor<_, ()> =
        ConnectionActor::new(queues, handle, Some(Box::pin(stream)), shutdown_token);
    let during = wireframe::connection::active_connection_count();
    assert_eq!(during, before + 1);

    let mut out = Vec::new();
    actor.run(&mut out).await.expect("actor run failed");
    let after = wireframe::connection::active_connection_count();
    assert_eq!(during - after, 1);
}
