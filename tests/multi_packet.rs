#![cfg(not(loom))]
//! Tests for multi-packet responses using channels.

use std::time::Duration;

use futures::TryStreamExt;
use rstest::{fixture, rstest};
use tokio::{sync::mpsc, task::yield_now, time::timeout};
use tokio_util::sync::CancellationToken;
use wireframe::{
    Response,
    connection::{ConnectionActor, FairnessConfig},
    push::{PushHandle, PushQueues},
};

#[derive(PartialEq, Debug)]
struct TestMsg(u8);

const CAPACITY: usize = 2;

/// Provide push queues, handle, and shutdown token for connection actor tests.
#[fixture]
fn actor_components() -> (PushQueues<u8>, PushHandle<u8>, CancellationToken) {
    let (queues, handle) = PushQueues::<u8>::builder()
        .high_capacity(4)
        .low_capacity(4)
        .build()
        .expect("failed to build PushQueues");
    (queues, handle, CancellationToken::new())
}

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
#[rstest(
    frames,
    case::empty(Vec::<u8>::new()),
    case::single(vec![42]),
    case::multiple(vec![11, 12, 13]),
)]
#[tokio::test]
async fn connection_actor_drains_multi_packet_channel(
    frames: Vec<u8>,
    actor_components: (PushQueues<u8>, PushHandle<u8>, CancellationToken),
) {
    let capacity = frames.len().max(1);
    let (tx, rx) = mpsc::channel(capacity);
    for &value in &frames {
        tx.send(value).await.expect("send frame");
    }
    drop(tx);

    let (queues, handle, shutdown) = actor_components;
    let mut actor: ConnectionActor<_, ()> = ConnectionActor::new(queues, handle, None, shutdown);
    actor.set_multi_packet(Some(rx));

    let mut out = Vec::new();
    actor.run(&mut out).await.expect("actor run failed");

    assert_eq!(out, frames);
}

#[rstest]
#[tokio::test]
async fn connection_actor_interleaves_multi_packet_and_priority_frames(
    actor_components: (PushQueues<u8>, PushHandle<u8>, CancellationToken),
) {
    let (queues, handle, shutdown) = actor_components;
    let multi_frames = [1_u8, 2, 3];
    let (multi_tx, multi_rx) = mpsc::channel(multi_frames.len());
    for &frame in &multi_frames {
        multi_tx.send(frame).await.expect("send multi-packet frame");
    }
    drop(multi_tx);

    handle
        .push_high_priority(10)
        .await
        .expect("push high-priority frame");
    handle
        .push_high_priority(11)
        .await
        .expect("push high-priority frame");
    handle
        .push_low_priority(100)
        .await
        .expect("push low-priority frame");
    handle
        .push_low_priority(101)
        .await
        .expect("push low-priority frame");

    let mut actor: ConnectionActor<_, ()> = ConnectionActor::new(queues, handle, None, shutdown);
    actor.set_fairness(FairnessConfig {
        max_high_before_low: 1,
        time_slice: None,
    });
    actor.set_multi_packet(Some(multi_rx));

    let mut out = Vec::new();
    actor.run(&mut out).await.expect("actor run failed");

    assert_eq!(out, vec![10, 100, 11, 101, 1, 2, 3]);
}

#[rstest]
#[tokio::test]
async fn shutdown_completes_multi_packet_channel(
    actor_components: (PushQueues<u8>, PushHandle<u8>, CancellationToken),
) {
    let (queues, handle, shutdown) = actor_components;
    let (tx, rx) = mpsc::channel(1);
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

    let out = timeout(Duration::from_millis(1000), join)
        .await
        .expect("actor shutdown timed out")
        .expect("actor task panicked");

    assert!(out.is_empty());
    drop(tx);
}

#[rstest]
#[tokio::test]
async fn shutdown_during_active_multi_packet_send(
    actor_components: (PushQueues<u8>, PushHandle<u8>, CancellationToken),
) {
    let (queues, handle, shutdown) = actor_components;
    let (tx, rx) = mpsc::channel(4);
    let mut actor: ConnectionActor<_, ()> = ConnectionActor::new(queues, handle, None, shutdown);
    actor.set_multi_packet(Some(rx));

    let cancel = actor.shutdown_token();

    let join = tokio::spawn(async move {
        let mut out = Vec::new();
        actor.run(&mut out).await.expect("actor run failed");
        out
    });

    tx.send(1).await.expect("send frame");
    tx.send(2).await.expect("send frame");
    yield_now().await;
    cancel.cancel();

    let _ = tx.send(3).await;

    let out = timeout(Duration::from_millis(1000), join)
        .await
        .expect("actor shutdown timed out")
        .expect("actor task panicked");
    assert!(out.is_empty() || out == vec![1, 2], "actor output: {out:?}");
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
