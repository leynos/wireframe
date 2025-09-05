//! Tests for multi-packet responses using channels.

use futures::StreamExt;
use tokio::sync::mpsc;
use wireframe::Response;

#[derive(PartialEq, Debug)]
struct TestMsg(u8);

/// Drain all messages from the stream.
async fn drain_all(stream: wireframe::FrameStream<TestMsg, ()>) -> Vec<TestMsg> {
    stream.map(|r| r.expect("stream error")).collect().await
}

/// Verifies that all messages sent through the channel are yielded by
/// `Response::MultiPacket`.
#[tokio::test]
async fn multi_packet_yields_messages() {
    let (tx, rx) = mpsc::channel(4);
    tx.send(TestMsg(1)).await.expect("send");
    tx.send(TestMsg(2)).await.expect("send");
    drop(tx);

    let resp: Response<TestMsg, ()> = Response::MultiPacket(rx);
    let received = drain_all(resp.into_stream()).await;
    assert_eq!(received, vec![TestMsg(1), TestMsg(2)]);
}

/// Yields no messages when the channel is immediately closed.
#[tokio::test]
async fn multi_packet_empty_channel() {
    let (tx, rx) = mpsc::channel(4);
    drop(tx);
    let resp: Response<TestMsg, ()> = Response::MultiPacket(rx);
    let received = drain_all(resp.into_stream()).await;
    assert!(received.is_empty());
}

/// Stops yielding when the sender is dropped before all messages are sent.
#[tokio::test]
async fn multi_packet_sender_dropped_before_all_messages() {
    let (tx, rx) = mpsc::channel(4);
    tx.send(TestMsg(1)).await.expect("send");
    drop(tx);
    let resp: Response<TestMsg, ()> = Response::MultiPacket(rx);
    let received = drain_all(resp.into_stream()).await;
    assert_eq!(received, vec![TestMsg(1)]);
}

/// Handles more messages than the channel capacity allows.
#[tokio::test]
async fn multi_packet_handles_channel_capacity() {
    let (tx, rx) = mpsc::channel(2);
    let send_task = tokio::spawn(async move {
        for i in 0..4u8 {
            tx.send(TestMsg(i)).await.expect("send");
        }
    });
    let resp: Response<TestMsg, ()> = Response::MultiPacket(rx);
    let received = drain_all(resp.into_stream()).await;
    send_task.await.expect("sender join");
    assert_eq!(
        received,
        vec![TestMsg(0), TestMsg(1), TestMsg(2), TestMsg(3)]
    );
}
