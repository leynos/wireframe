//! Tests for streams generated with the `async-stream` crate.
//!
//! These ensure that a `ConnectionActor` correctly drains frames from an
//! async-stream based `FrameStream`.

use async_stream::try_stream;
use rstest::rstest;
use tokio_util::sync::CancellationToken;
use wireframe::{
    connection::ConnectionActor,
    push::PushQueues,
    response::{FrameStream, WireframeError},
};

fn frame_stream() -> impl futures::Stream<Item = Result<u8, WireframeError>> {
    try_stream! {
        for n in 0u8..3 {
            yield n;
        }
    }
}

#[rstest]
#[tokio::test]
async fn async_stream_frames_processed_in_order() {
    let (queues, handle) = PushQueues::builder()
        .high_capacity(8)
        .low_capacity(8)
        .build()
        .expect("failed to build PushQueues");
    let shutdown = CancellationToken::new();
    let stream: FrameStream<u8> = Box::pin(frame_stream());

    let mut actor = ConnectionActor::new(queues, handle, Some(stream), shutdown);
    let mut out = Vec::new();
    actor.run(&mut out).await.expect("actor run failed");
    assert_eq!(out, vec![0, 1, 2]);
}
