//! Tests for multi-packet responses using channels.

use rstest::rstest;
use tokio::sync::mpsc;
use wireframe::Response;
use wireframe_testing::collect_multi_packet;

#[derive(PartialEq, Debug)]
struct TestMsg(u8);

const CAPACITY: usize = 2;

/// `collect_multi_packet` drains every frame regardless of channel state.
///
/// This covers empty channels, partial sends, and when senders outpace the channel's
/// capacity.
#[rstest(count, case(0), case(1), case(2), case(CAPACITY + 1))]
#[tokio::test]
<<<<<<< HEAD
async fn multi_packet_empty_channel() {
    let (tx, rx) = mpsc::channel(4);
    drop(tx);
    let resp: Response<TestMsg, ()> = Response::MultiPacket(rx);
    let received = collect_multi_packet(resp).await;
    assert!(received.is_empty());
}

/// Stops yielding when the sender is dropped before all messages are sent.
#[tokio::test]
async fn multi_packet_sender_dropped_before_all_messages() {
    let (tx, rx) = mpsc::channel(4);
    tx.send(TestMsg(1)).await.expect("send");
    drop(tx);
    let resp: Response<TestMsg, ()> = Response::MultiPacket(rx);
    let received = collect_multi_packet(resp).await;
    assert_eq!(received, vec![TestMsg(1)]);
}

/// Test that sending fails after the receiver is dropped.
#[tokio::test]
async fn multi_packet_send_fails_after_receiver_dropped() {
    let (tx, rx) = mpsc::channel::<TestMsg>(2);
    drop(rx);
    let error = tx
        .send(TestMsg(42))
        .await
        .expect_err("Send should fail when receiver is dropped");
    let mpsc::error::SendError(msg) = error;
    assert_eq!(msg, TestMsg(42));
}

/// Handles more messages than the channel capacity allows.
#[tokio::test]
async fn multi_packet_handles_channel_capacity() {
    let (tx, rx) = mpsc::channel(2);
||||||| parent of 047215a (Parameterize multi-packet tests)
async fn multi_packet_empty_channel() {
    let (tx, rx) = mpsc::channel(4);
    drop(tx);
    let resp: Response<TestMsg, ()> = Response::MultiPacket(rx);
    let received = collect_multi_packet(resp).await;
    assert!(received.is_empty());
}

/// Stops yielding when the sender is dropped before all messages are sent.
#[tokio::test]
async fn multi_packet_sender_dropped_before_all_messages() {
    let (tx, rx) = mpsc::channel(4);
    tx.send(TestMsg(1)).await.expect("send");
    drop(tx);
    let resp: Response<TestMsg, ()> = Response::MultiPacket(rx);
    let received = collect_multi_packet(resp).await;
    assert_eq!(received, vec![TestMsg(1)]);
}

/// Handles more messages than the channel capacity allows.
#[tokio::test]
async fn multi_packet_handles_channel_capacity() {
    let (tx, rx) = mpsc::channel(2);
=======
async fn multi_packet_drains_all_messages(count: usize) {
    let (tx, rx) = mpsc::channel(CAPACITY);
>>>>>>> 047215a (Parameterize multi-packet tests)
    let send_task = tokio::spawn(async move {
        for i in 0..count {
            tx.send(TestMsg(u8::try_from(i).expect("<= u8::MAX")))
                .await
                .expect("send");
        }
    });
    let resp: Response<TestMsg, ()> = Response::MultiPacket(rx);
    let received = collect_multi_packet(resp).await;
>>>>>>> 5e27dd8 (Add MultiPacket test helper implementation)
    send_task.await.expect("sender join");
    assert_eq!(
        received,
        (0..count)
            .map(|i| TestMsg(u8::try_from(i).expect("<= u8::MAX")))
            .collect::<Vec<_>>()
    );
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
