#![cfg(not(loom))]
//! Tests for multi-packet responses using channels.

use futures::TryStreamExt;
use rstest::rstest;
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;
use wireframe::{Response, connection::ConnectionActor, push::PushQueues};
use wireframe_testing::collect_multi_packet;

#[derive(PartialEq, Debug)]
struct TestMsg(u8);

const CAPACITY: usize = 2;

/// Drain all messages from a `FrameStream` for non-channel response variants.
async fn drain_all<F, E: std::fmt::Debug>(stream: wireframe::FrameStream<F, E>) -> Vec<F> {
    stream.try_collect::<Vec<_>>().await.expect("stream error")
}

/// `collect_multi_packet` drains every frame regardless of channel state.
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
    let received = collect_multi_packet(resp).await;
    send_task.await.expect("sender join");
    assert_eq!(
        received,
        (0..count)
            .map(|i| TestMsg(u8::try_from(i).expect("<= u8::MAX")))
            .collect::<Vec<_>>()
    );
}

/// Drains frames from a multi-packet channel via the connection actor.
#[tokio::test]
async fn connection_actor_drains_multi_packet_channel() {
    let frames = vec![11u8, 12, 13];
    let (tx, rx) = mpsc::channel(frames.len());
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
