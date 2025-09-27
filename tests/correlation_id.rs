#![cfg(not(loom))]
//! Tests for `correlation_id` propagation in streaming responses.
use async_stream::try_stream;
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;
use wireframe::{
    app::{Envelope, Packet},
    connection::ConnectionActor,
    push::PushQueues,
    response::FrameStream,
};

#[tokio::test]
async fn stream_frames_carry_request_correlation_id() {
    let cid = 42u64;
    let stream: FrameStream<Envelope> = Box::pin(try_stream! {
        yield Envelope::new(1, Some(cid), vec![1]);
        yield Envelope::new(1, Some(cid), vec![2]);
    });
    let (queues, handle) = PushQueues::<Envelope>::builder()
        .high_capacity(1)
        .low_capacity(1)
        .unlimited()
        .build()
        .expect("failed to build PushQueues");
    let shutdown = CancellationToken::new();
    let mut actor = ConnectionActor::new(queues, handle, Some(stream), shutdown);
    let mut out = Vec::new();
    actor.run(&mut out).await.expect("actor run failed");
    assert!(out.iter().all(|e| e.correlation_id() == Some(cid)));
}

#[tokio::test]
async fn multi_packet_frames_carry_request_correlation_id() {
    let cid = 7u64;
    let (tx, rx) = mpsc::channel(4);
    tx.send(Envelope::new(1, None, vec![1]))
        .await
        .expect("send frame");
    tx.send(Envelope::new(1, Some(99), vec![2]))
        .await
        .expect("send frame");
    drop(tx);

    let (queues, handle) = PushQueues::<Envelope>::builder()
        .high_capacity(2)
        .low_capacity(2)
        .unlimited()
        .build()
        .expect("failed to build PushQueues");
    let shutdown = CancellationToken::new();
    let mut actor: ConnectionActor<Envelope, ()> =
        ConnectionActor::new(queues, handle, None, shutdown);
    actor.set_multi_packet_with_correlation(Some(rx), Some(cid));

    let mut out = Vec::new();
    actor.run(&mut out).await.expect("actor run failed");

    assert!(out.iter().all(|frame| frame.correlation_id() == Some(cid)));
}
