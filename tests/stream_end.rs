//! Tests for explicit end-of-stream signalling.
use std::sync::Arc;

use async_stream::try_stream;
use rstest::rstest;
use tokio_util::sync::CancellationToken;
use wireframe::{
    connection::ConnectionActor,
    hooks::{ConnectionContext, ProtocolHooks, WireframeProtocol},
    push::PushQueues,
    response::FrameStream,
};

struct TerminatorProto;

impl WireframeProtocol for TerminatorProto {
    type Frame = u8;
    type ProtocolError = ();

    fn stream_end_frame(&self, _ctx: &mut ConnectionContext) -> Option<Self::Frame> { Some(0) }
}

#[rstest]
#[tokio::test]
async fn emits_end_frame() {
    let stream: FrameStream<u8> = Box::pin(try_stream! {
        yield 1;
        yield 2;
    });

    let (queues, handle) = PushQueues::bounded(1, 1);
    let shutdown = CancellationToken::new();
    let hooks = ProtocolHooks::from_protocol(&Arc::new(TerminatorProto));
    let mut actor = ConnectionActor::with_hooks(queues, handle, Some(stream), shutdown, hooks);

    let mut out = Vec::new();
    actor.run(&mut out).await.expect("actor run failed");

    assert_eq!(out, vec![1, 2, 0]);
}
