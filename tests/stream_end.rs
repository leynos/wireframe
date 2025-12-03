//! Tests for explicit end-of-stream signalling.
#![cfg(not(loom))]

mod support;

use std::sync::Arc;

use async_stream::try_stream;
use rstest::{fixture, rstest};
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;
use wireframe::{
    connection::{ConnectionActor, ConnectionChannels},
    hooks::{ConnectionContext, ProtocolHooks, WireframeProtocol},
    push::{PushHandle, PushQueues},
    response::FrameStream,
};

#[path = "common/terminator.rs"]
mod terminator;
use terminator::Terminator;

type TestResult<T = ()> = Result<T, Box<dyn std::error::Error + Send + Sync>>;

#[fixture]
fn queues() -> Result<(PushQueues<u8>, PushHandle<u8>), wireframe::push::PushConfigError> {
    support::builder::<u8>().build()
}

#[rstest]
#[tokio::test]
async fn emits_end_frame(
    queues: Result<(PushQueues<u8>, PushHandle<u8>), wireframe::push::PushConfigError>,
) -> TestResult<()> {
    let (queues, handle) = queues?;
    // fixture injected above
    let stream: FrameStream<u8> = Box::pin(try_stream! {
        yield 1;
        yield 2;
    });
    let shutdown = CancellationToken::new();
    let hooks = ProtocolHooks::from_protocol(&Arc::new(Terminator));
    let mut actor = ConnectionActor::with_hooks(
        ConnectionChannels::new(queues, handle),
        Some(stream),
        shutdown,
        hooks,
    );

    let mut out = Vec::new();
    actor
        .run(&mut out)
        .await
        .map_err(|e| std::io::Error::other(format!("connection actor failed: {e:?}")))?;

    if out != vec![1, 2, 0] {
        return Err("unexpected output frames".into());
    }
    Ok(())
}

#[rstest]
#[tokio::test]
async fn multi_packet_emits_end_frame(
    queues: Result<(PushQueues<u8>, PushHandle<u8>), wireframe::push::PushConfigError>,
) -> TestResult<()> {
    let (queues, handle) = queues?;
    let (tx, rx) = mpsc::channel(4);
    tx.send(1)
        .await
        .map_err(|e| std::io::Error::other(format!("send frame: {e}")))?;
    tx.send(2)
        .await
        .map_err(|e| std::io::Error::other(format!("send frame: {e}")))?;
    drop(tx);

    let shutdown = CancellationToken::new();
    let hooks = ProtocolHooks::from_protocol(&Arc::new(Terminator));
    let mut actor = ConnectionActor::with_hooks(
        ConnectionChannels::new(queues, handle),
        None,
        shutdown,
        hooks,
    );
    actor.set_multi_packet(Some(rx));

    let mut out = Vec::new();
    actor
        .run(&mut out)
        .await
        .map_err(|e| std::io::Error::other(format!("connection actor failed: {e:?}")))?;

    if out != vec![1, 2, 0] {
        return Err("unexpected output frames".into());
    }
    Ok(())
}

#[rstest]
#[tokio::test]
async fn multi_packet_respects_no_terminator(
    queues: Result<(PushQueues<u8>, PushHandle<u8>), wireframe::push::PushConfigError>,
) -> TestResult<()> {
    struct NoTerminator;

    impl WireframeProtocol for NoTerminator {
        type Frame = u8;
        type ProtocolError = ();

        fn stream_end_frame(&self, _ctx: &mut ConnectionContext) -> Option<Self::Frame> { None }
    }

    let (queues, handle) = queues?;
    let (tx, rx) = mpsc::channel(2);
    tx.send(9)
        .await
        .map_err(|e| std::io::Error::other(format!("send frame: {e}")))?;
    drop(tx);

    let shutdown = CancellationToken::new();
    let hooks = ProtocolHooks::from_protocol(&Arc::new(NoTerminator));
    let mut actor = ConnectionActor::with_hooks(
        ConnectionChannels::new(queues, handle),
        None,
        shutdown,
        hooks,
    );
    actor.set_multi_packet(Some(rx));

    let mut out = Vec::new();
    actor
        .run(&mut out)
        .await
        .map_err(|e| std::io::Error::other(format!("connection actor failed: {e:?}")))?;

    if out != vec![9] {
        return Err("unexpected output frames".into());
    }
    Ok(())
}

#[rstest]
#[tokio::test]
async fn multi_packet_empty_channel_emits_end(
    queues: Result<(PushQueues<u8>, PushHandle<u8>), wireframe::push::PushConfigError>,
) -> TestResult<()> {
    let (queues, handle) = queues?;
    let (tx, rx) = mpsc::channel(1);
    drop(tx);

    let shutdown = CancellationToken::new();
    let hooks = ProtocolHooks::from_protocol(&Arc::new(Terminator));
    let mut actor = ConnectionActor::with_hooks(
        ConnectionChannels::new(queues, handle),
        None,
        shutdown,
        hooks,
    );
    actor.set_multi_packet(Some(rx));

    let mut out = Vec::new();
    actor
        .run(&mut out)
        .await
        .map_err(|e| std::io::Error::other(format!("connection actor failed: {e:?}")))?;

    if out != vec![0] {
        return Err("unexpected output frames".into());
    }
    Ok(())
}

#[rstest]
#[tokio::test]
async fn multi_packet_empty_channel_no_terminator_emits_nothing(
    queues: Result<(PushQueues<u8>, PushHandle<u8>), wireframe::push::PushConfigError>,
) -> TestResult<()> {
    struct NoTerminator;

    impl WireframeProtocol for NoTerminator {
        type Frame = u8;
        type ProtocolError = ();

        fn stream_end_frame(&self, _ctx: &mut ConnectionContext) -> Option<Self::Frame> { None }
    }

    let (queues, handle) = queues?;
    let (tx, rx) = mpsc::channel(1);
    drop(tx);

    let shutdown = CancellationToken::new();
    let hooks = ProtocolHooks::from_protocol(&Arc::new(NoTerminator));
    let mut actor = ConnectionActor::with_hooks(
        ConnectionChannels::new(queues, handle),
        None,
        shutdown,
        hooks,
    );
    actor.set_multi_packet(Some(rx));

    let mut out = Vec::new();
    actor
        .run(&mut out)
        .await
        .map_err(|e| std::io::Error::other(format!("connection actor failed: {e:?}")))?;

    if !out.is_empty() {
        return Err("expected no frames".into());
    }
    Ok(())
}

#[rstest]
#[tokio::test]
async fn emits_no_end_frame_when_none(
    queues: Result<(PushQueues<u8>, PushHandle<u8>), wireframe::push::PushConfigError>,
) -> TestResult<()> {
    struct NoTerminator;

    impl WireframeProtocol for NoTerminator {
        type Frame = u8;
        type ProtocolError = ();

        fn stream_end_frame(&self, _ctx: &mut ConnectionContext) -> Option<Self::Frame> { None }
    }

    let (queues, handle) = queues?;
    // fixture injected above
    let stream: FrameStream<u8> = Box::pin(try_stream! {
        yield 7;
        yield 8;
    });

    let shutdown = CancellationToken::new();
    let hooks = ProtocolHooks::from_protocol(&Arc::new(NoTerminator));
    let mut actor = ConnectionActor::with_hooks(
        ConnectionChannels::new(queues, handle),
        Some(stream),
        shutdown,
        hooks,
    );

    let mut out = Vec::new();
    actor
        .run(&mut out)
        .await
        .map_err(|e| std::io::Error::other(format!("connection actor failed: {e:?}")))?;

    if out != vec![7, 8] {
        return Err("unexpected frames".into());
    }
    Ok(())
}
