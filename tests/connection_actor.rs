use futures::stream;
use rstest::{fixture, rstest};
use tokio_util::sync::CancellationToken;
use wireframe::{connection::ConnectionActor, push::PushQueues};

#[fixture]
#[allow(unused_braces)]
fn queues() -> (PushQueues<u8>, wireframe::push::PushHandle<u8>) { PushQueues::bounded(8, 8) }

#[fixture]
#[allow(unused_braces)]
fn shutdown_token() -> CancellationToken { CancellationToken::new() }

#[rstest]
#[tokio::test]
async fn high_priority_before_low_and_stream(
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
async fn shutdown_terminates_actor(
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
async fn actor_processes_until_sources_close(
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
