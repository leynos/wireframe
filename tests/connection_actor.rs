//! Tests for the `ConnectionActor` component.
//!
//! These cover priority order, shutdown behaviour, error propagation,
//! interleaved cancellation and back-pressure handling.

use futures::stream;
use rstest::{fixture, rstest};
use tokio::time::{Duration, sleep, timeout};
use tokio_util::sync::CancellationToken;
use wireframe::{
    connection::ConnectionActor,
    push::PushQueues,
    response::{FrameStream, WireframeError},
};

#[fixture]
#[allow(unused_braces)]
fn queues() -> (PushQueues<u8>, wireframe::push::PushHandle<u8>) { PushQueues::bounded(8, 8) }

#[fixture]
#[allow(unused_braces)]
fn shutdown_token() -> CancellationToken { CancellationToken::new() }

#[fixture]
#[allow(unused_braces)]
fn empty_stream() -> Option<FrameStream<u8, ()>> { None }

#[rstest]
#[tokio::test]
async fn strict_priority_order(
    queues: (PushQueues<u8>, wireframe::push::PushHandle<u8>),
    shutdown_token: CancellationToken,
) {
    let (queues, handle) = queues;
    handle.push_low_priority(2).await.unwrap();
    handle.push_high_priority(1).await.unwrap();
    drop(handle);

    let stream = stream::iter(vec![Ok(3u8)]);
    let mut actor: ConnectionActor<_, ()> =
        ConnectionActor::new(queues, Some(Box::pin(stream)), shutdown_token);
    let mut out = Vec::new();
    actor.run(&mut out).await.unwrap();
    assert_eq!(out, vec![1, 2, 3]);
}

#[rstest]
#[tokio::test]
async fn shutdown_signal_precedence(
    queues: (PushQueues<u8>, wireframe::push::PushHandle<u8>),
    shutdown_token: CancellationToken,
) {
    let (queues, _handle) = queues;
    shutdown_token.cancel();
    let mut actor: ConnectionActor<_, ()> = ConnectionActor::new(queues, None, shutdown_token);
    let mut out = Vec::new();
    actor.run(&mut out).await.unwrap();
    assert!(out.is_empty());
}

#[rstest]
#[tokio::test]
async fn complete_draining_of_sources(
    queues: (PushQueues<u8>, wireframe::push::PushHandle<u8>),
    shutdown_token: CancellationToken,
) {
    let (queues, handle) = queues;
    handle.push_high_priority(1).await.unwrap();
    drop(handle);

    let stream = stream::iter(vec![Ok(2u8), Ok(3u8)]);
    let mut actor: ConnectionActor<_, ()> =
        ConnectionActor::new(queues, Some(Box::pin(stream)), shutdown_token);
    let mut out = Vec::new();
    actor.run(&mut out).await.unwrap();
    assert_eq!(out, vec![1, 2, 3]);
}

#[derive(Debug)]
enum TestError {
    Kaboom,
}

#[rstest]
#[tokio::test]
async fn error_propagation_from_stream(
    queues: (PushQueues<u8>, wireframe::push::PushHandle<u8>),
    shutdown_token: CancellationToken,
) {
    let (queues, _handle) = queues;
    let stream = stream::iter(vec![
        Ok(1u8),
        Ok(2u8),
        Err(WireframeError::Protocol(TestError::Kaboom)),
    ]);
    let mut actor: ConnectionActor<_, TestError> =
        ConnectionActor::new(queues, Some(Box::pin(stream)), shutdown_token);
    let mut out = Vec::new();
    let result = actor.run(&mut out).await;
    assert!(matches!(
        result,
        Err(WireframeError::Protocol(TestError::Kaboom))
    ));
    assert_eq!(out, vec![1, 2]);
}

#[rstest]
#[tokio::test]
async fn interleaved_shutdown_during_stream(
    queues: (PushQueues<u8>, wireframe::push::PushHandle<u8>),
    shutdown_token: CancellationToken,
) {
    let (queues, _handle) = queues;
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
        ConnectionActor::new(queues, Some(Box::pin(stream)), shutdown_token);
    let mut out = Vec::new();
    actor.run(&mut out).await.unwrap();
    assert!(!out.is_empty() && out.len() < 5);
}

#[rstest]
#[tokio::test]
async fn push_queue_exhaustion_backpressure() {
    let (mut queues, handle) = PushQueues::bounded(1, 1);
    handle.push_high_priority(1).await.unwrap();

    let blocked = timeout(Duration::from_millis(50), handle.push_high_priority(2)).await;
    assert!(blocked.is_err());

    // clean up without exposing internal fields
    queues.close();
}
