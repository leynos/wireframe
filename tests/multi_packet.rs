#![cfg(not(loom))]
//! Tests for multi-packet responses using channels.

use std::time::Duration;

use futures::TryStreamExt;
use rstest::rstest;
use tokio::{sync::mpsc, task::yield_now, time::timeout};
use tokio_util::sync::CancellationToken;
use wireframe::{Response, connection::ConnectionActor, push::PushQueues};

#[derive(PartialEq, Debug)]
struct TestMsg(u8);

const CAPACITY: usize = 2;

/// Drain all messages from a `FrameStream` for non-channel response variants.
async fn drain_all<F, E: std::fmt::Debug>(stream: wireframe::FrameStream<F, E>) -> Vec<F> {
    stream.try_collect::<Vec<_>>().await.expect("stream error")
}

/// Multi-packet responses drain every frame regardless of channel state.
///
/// This covers empty channels, partial sends, and when senders outpace the
/// channel's capacity.
#[rstest(count, case(0), case(1), case(2), case(CAPACITY + 1))]
#[tokio::test]
async fn multi_packet_drains_all_messages(count: usize) {
    let (tx, rx) = mpsc::channel(CAPACITY);
    let send_task = tokio::spawn(async move {
        for i in 0..count {
            tx.send(TestMsg(u8::try_from(i).expect("<= u8::MAX")))
                .await
                .expect("send");
        }
    });
    let resp: Response<TestMsg, ()> = Response::MultiPacket(rx);
    let received = drain_all(resp.into_stream()).await;
    send_task.await.expect("sender join");
    assert_eq!(
        received,
        (0..count)
            .map(|i| TestMsg(u8::try_from(i).expect("<= u8::MAX")))
            .collect::<Vec<_>>()
    );
}

/// Drains frames from a multi-packet channel via the connection actor.
#[rstest]
#[case::empty(Vec::<u8>::new())]
#[case::single(vec![42])]
#[case::multiple(vec![11, 12, 13])]
#[tokio::test]
async fn connection_actor_drains_multi_packet_channel(#[case] frames: Vec<u8>) {
    let capacity = frames.len().max(1);
    let (tx, rx) = mpsc::channel(capacity);
    for &value in &frames {
        tx.send(value).await.expect("send frame");
    }
    drop(tx);

    let (queues, handle) = PushQueues::<u8>::builder()
        .high_capacity(4)
        .low_capacity(4)
        .build()
        .expect("failed to build PushQueues");
    let shutdown = CancellationToken::new();
    let mut actor: ConnectionActor<_, ()> = ConnectionActor::new(queues, handle, None, shutdown);
    actor.set_multi_packet(Some(rx));

    let mut out = Vec::new();
    actor.run(&mut out).await.expect("actor run failed");

    assert_eq!(out, frames);
}

#[tokio::test]
async fn shutdown_completes_multi_packet_channel() {
    let (queues, handle) = PushQueues::<u8>::builder()
        .high_capacity(4)
        .low_capacity(4)
        .build()
        .expect("failed to build PushQueues");
    let (tx, rx) = mpsc::channel(1);
    let shutdown = CancellationToken::new();
    let mut actor: ConnectionActor<_, ()> = ConnectionActor::new(queues, handle, None, shutdown);
    actor.set_multi_packet(Some(rx));

    let cancel = actor.shutdown_token();

    let join = tokio::spawn(async move {
        let mut out = Vec::new();
        actor.run(&mut out).await.expect("actor run failed");
        out
    });

    yield_now().await;
    cancel.cancel();

    let out = timeout(Duration::from_millis(250), join)
        .await
        .expect("actor shutdown timed out")
        .expect("actor task panicked");

    assert!(out.is_empty());
    drop(tx);
}

/// Returns an empty stream for an empty vector response.
#[tokio::test]
async fn vec_empty_returns_empty_stream() {
    let resp: Response<TestMsg, ()> = Response::Vec(Vec::new());
    let received = drain_all(resp.into_stream()).await;
    assert!(received.is_empty());
}

/// `Response::Empty` yields no frames.
#[tokio::test]
async fn empty_returns_empty_stream() {
    let resp: Response<TestMsg, ()> = Response::Empty;
    let received = drain_all(resp.into_stream()).await;
    assert!(received.is_empty());
}
