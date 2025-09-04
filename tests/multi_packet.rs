//! Tests for multi-packet responses using channels.

use tokio::sync::mpsc;
use wireframe::Response;

#[derive(bincode::Encode, bincode::BorrowDecode, PartialEq, Debug)]
struct TestMsg(u8);

/// Verifies that all messages sent through the channel are yielded by `Response::MultiPacket`.
#[tokio::test]
async fn multi_packet_yields_messages() {
    let (tx, rx) = mpsc::channel(4);
    tx.send(TestMsg(1)).await.expect("send");
    tx.send(TestMsg(2)).await.expect("send");
    drop(tx);

    let resp: Response<TestMsg, ()> = Response::MultiPacket(rx);
    let mut received = Vec::new();
    if let Response::MultiPacket(mut rx) = resp {
        while let Some(msg) = rx.recv().await {
            received.push(msg);
        }
    }
    assert_eq!(received, vec![TestMsg(1), TestMsg(2)]);
}
