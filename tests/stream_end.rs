#![cfg(not(loom))]
//! Tests for explicit end-of-stream signalling.

mod support;

use std::sync::Arc;

use async_stream::try_stream;
use rstest::{fixture, rstest};
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;
use wireframe::{
    connection::ConnectionActor,
    hooks::{ConnectionContext, ProtocolHooks, WireframeProtocol},
    push::{PushHandle, PushQueues},
    response::FrameStream,
};

#[path = "common/terminator.rs"]
mod terminator;
use terminator::Terminator;

#[fixture]
fn queues() -> (PushQueues<u8>, PushHandle<u8>) {
    support::builder::<u8>()
        .build()
        .expect("failed to build PushQueues")
}

#[rstest]
#[tokio::test]
async fn emits_end_frame(queues: (PushQueues<u8>, PushHandle<u8>)) {
    let (queues, handle) = queues;
    // fixture injected above
    let stream: FrameStream<u8> = Box::pin(try_stream! {
        yield 1;
        yield 2;
    });
    let shutdown = CancellationToken::new();
    let hooks = ProtocolHooks::from_protocol(&Arc::new(Terminator));
    let mut actor = ConnectionActor::with_hooks(queues, handle, Some(stream), shutdown, hooks);

    let mut out = Vec::new();
    actor.run(&mut out).await.expect("actor run failed");

    assert_eq!(out, vec![1, 2, 0]);
}

#[rstest]
#[tokio::test]
async fn multi_packet_emits_end_frame(queues: (PushQueues<u8>, PushHandle<u8>)) {
    let (queues, handle) = queues;
    let (tx, rx) = mpsc::channel(4);
    tx.send(1).await.expect("send frame");
    tx.send(2).await.expect("send frame");
    drop(tx);

    let shutdown = CancellationToken::new();
    let hooks = ProtocolHooks::from_protocol(&Arc::new(Terminator));
    let mut actor = ConnectionActor::with_hooks(queues, handle, None, shutdown, hooks);
    actor.set_multi_packet(Some(rx));

    let mut out = Vec::new();
    actor.run(&mut out).await.expect("actor run failed");

    assert_eq!(out, vec![1, 2, 0]);
}

#[rstest]
#[tokio::test]
async fn multi_packet_respects_no_terminator(queues: (PushQueues<u8>, PushHandle<u8>)) {
    struct NoTerminator;

    impl WireframeProtocol for NoTerminator {
        type Frame = u8;
        type ProtocolError = ();

        fn stream_end_frame(&self, _ctx: &mut ConnectionContext) -> Option<Self::Frame> { None }
    }

    let (queues, handle) = queues;
    let (tx, rx) = mpsc::channel(2);
    tx.send(9).await.expect("send frame");
    drop(tx);

    let shutdown = CancellationToken::new();
    let hooks = ProtocolHooks::from_protocol(&Arc::new(NoTerminator));
    let mut actor = ConnectionActor::with_hooks(queues, handle, None, shutdown, hooks);
    actor.set_multi_packet(Some(rx));

    let mut out = Vec::new();
    actor.run(&mut out).await.expect("actor run failed");

    assert_eq!(out, vec![9]);
}

#[rstest]
#[tokio::test]
async fn multi_packet_empty_channel_emits_end(queues: (PushQueues<u8>, PushHandle<u8>)) {
    let (queues, handle) = queues;
    let (tx, rx) = mpsc::channel(1);
    drop(tx);

    let shutdown = CancellationToken::new();
    let hooks = ProtocolHooks::from_protocol(&Arc::new(Terminator));
    let mut actor = ConnectionActor::with_hooks(queues, handle, None, shutdown, hooks);
    actor.set_multi_packet(Some(rx));

    let mut out = Vec::new();
    actor.run(&mut out).await.expect("actor run failed");

    assert_eq!(out, vec![0]);
}

#[rstest]
#[tokio::test]
async fn emits_no_end_frame_when_none(queues: (PushQueues<u8>, PushHandle<u8>)) {
    struct NoTerminator;

    impl WireframeProtocol for NoTerminator {
        type Frame = u8;
        type ProtocolError = ();

        fn stream_end_frame(&self, _ctx: &mut ConnectionContext) -> Option<Self::Frame> { None }
    }

    let (queues, handle) = queues;
    // fixture injected above
    let stream: FrameStream<u8> = Box::pin(try_stream! {
        yield 7;
        yield 8;
    });

    let shutdown = CancellationToken::new();
    let hooks = ProtocolHooks::from_protocol(&Arc::new(NoTerminator));
    let mut actor = ConnectionActor::with_hooks(queues, handle, Some(stream), shutdown, hooks);

    let mut out = Vec::new();
    actor.run(&mut out).await.expect("actor run failed");

    assert_eq!(out, vec![7, 8]);
}
