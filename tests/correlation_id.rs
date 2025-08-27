//! Tests for `correlation_id` propagation in streaming responses.
use async_stream::try_stream;
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
    let (queues, handle) = PushQueues::builder()
        .high_capacity(1)
        .low_capacity(1)
        .build()
        .expect("failed to build push queues");
    let shutdown = CancellationToken::new();
    let mut actor = ConnectionActor::new(queues, handle, Some(stream), shutdown);
    let mut out = Vec::new();
    actor.run(&mut out).await.expect("actor run failed");
    assert!(out.iter().all(|e| e.correlation_id() == Some(cid)));
}
