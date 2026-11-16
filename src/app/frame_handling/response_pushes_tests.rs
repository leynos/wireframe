//! The handler keeps being polled while a push write waits for the transport.
//!
//! A peer that stops reading without closing leaves a push write pending while
//! the peer is still observed, so neither the server stop token nor the idle
//! grace bounds it. This module proves the handler is polled during that wait,
//! that a response it produces is returned once the write completes, and that
//! an abandoned write still stops the connection.

use std::{
    future::Future,
    io,
    pin::Pin,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    task::{Context, Poll},
};

use futures::Sink;
use tokio::sync::oneshot;
use tokio_util::sync::CancellationToken;

use super::{ResponseContext, response_pushes::run_handler_with_pushes};
use crate::{
    app::{Envelope, codec_driver::FramePipeline},
    connection::{ConnectionActor, ConnectionChannels, UntrackedActorInit},
    hooks::{ConnectionContext, ProtocolHooks},
    push::{PushConfigError, PushHandle, PushQueues},
    serializer::BincodeSerializer,
    test_helpers::frame_codec::{TestCodec, TestFrame},
};

/// Maximum frame length accepted by the test codec.
const MAX_FRAME_LENGTH: usize = 1024;

/// Response the deferred handler returns once it is polled twice.
const HANDLER_RESPONSE: &str = "handler finished";

/// Actor fixture paired with the push handle that keeps its queues open.
type ActorFixture = (
    ConnectionActor<Envelope, (), TestFrame>,
    PushHandle<TestFrame>,
);

/// Build an untracked actor whose push queues stay open for the test.
///
/// # Errors
///
/// Returns [`PushConfigError`] if the queue capacities are rejected.
fn build_actor(shutdown: CancellationToken) -> Result<ActorFixture, PushConfigError> {
    let (queues, handle) = PushQueues::<TestFrame>::builder()
        .high_capacity(1)
        .low_capacity(1)
        .build()?;
    let actor = ConnectionActor::new_untracked(UntrackedActorInit {
        channels: ConnectionChannels::new(queues, handle.clone()),
        response: None,
        shutdown,
        hooks: ProtocolHooks::default(),
        ctx: ConnectionContext::default(),
    });
    Ok((actor, handle))
}

/// The push frame the test queues before running the handler.
fn queued_push() -> TestFrame {
    TestFrame {
        tag: 7,
        payload: vec![7],
    }
}

/// Sink whose writes stay pending until the test releases them.
struct BlockingSink {
    /// Fires when the test lets the transport accept the pending write.
    release: oneshot::Receiver<()>,
    /// Whether the release signal has been observed.
    released: bool,
    /// Frames the transport accepted, in write order.
    written: Vec<TestFrame>,
}

impl BlockingSink {
    /// Build a sink that blocks until `release` fires.
    const fn new(release: oneshot::Receiver<()>) -> Self {
        Self {
            release,
            released: false,
            written: Vec::new(),
        }
    }
}

impl Sink<TestFrame> for BlockingSink {
    type Error = io::Error;

    fn poll_ready(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        if !self.released {
            match Pin::new(&mut self.release).poll(cx) {
                Poll::Ready(Ok(()) | Err(_)) => self.released = true,
                Poll::Pending => return Poll::Pending,
            }
        }
        Poll::Ready(Ok(()))
    }

    fn start_send(mut self: Pin<&mut Self>, item: TestFrame) -> Result<(), Self::Error> {
        self.written.push(item);
        Ok(())
    }

    fn poll_flush(self: Pin<&mut Self>, _cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        Poll::Ready(Ok(()))
    }

    fn poll_close(self: Pin<&mut Self>, _cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        Poll::Ready(Ok(()))
    }
}

/// Handler future that completes only once it has been polled twice.
///
/// The first poll is what the response loop performs before it drains a push
/// lane, so producing the response only on the second poll proves the handler
/// is polled again while the push write waits.
struct DeferredHandler {
    /// Set when the handler produces its response.
    finished: Arc<AtomicBool>,
    /// Number of times the future has been polled.
    polls: usize,
}

impl DeferredHandler {
    /// Build a handler that reports completion through `finished`.
    const fn new(finished: Arc<AtomicBool>) -> Self { Self { finished, polls: 0 } }
}

impl Future for DeferredHandler {
    type Output = &'static str;

    fn poll(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        self.polls += 1;
        if self.polls < 2 {
            cx.waker().wake_by_ref();
            return Poll::Pending;
        }
        self.finished.store(true, Ordering::SeqCst);
        Poll::Ready(HANDLER_RESPONSE)
    }
}

/// A push write waiting on the peer must not suspend the handler.
///
/// The peer is still observed, so the write has neither the stop token nor the
/// idle grace as a bound. Awaiting it alone would stop polling the handler
/// until shutdown, which would suppress a handler timeout or recovery path.
#[tokio::test]
async fn the_handler_keeps_running_while_a_push_write_waits() {
    let (mut actor, handle) = build_actor(CancellationToken::new()).expect("actor queues build");
    handle
        .push_high_priority(queued_push())
        .await
        .expect("the lane has capacity");
    let (release, blocked) = oneshot::channel();
    let mut sink = BlockingSink::new(blocked);
    let codec = TestCodec::new(MAX_FRAME_LENGTH);
    let serializer = BincodeSerializer;
    let mut pipeline: FramePipeline<TestFrame> = FramePipeline::new(None);
    let mut ctx = ResponseContext {
        serializer: &serializer,
        sink: &mut sink,
        pipeline: &mut pipeline,
        codec: &codec,
        actor: &mut actor,
        protocol: None,
    };
    let finished = Arc::new(AtomicBool::new(false));
    let call = DeferredHandler::new(Arc::clone(&finished));

    let response = {
        let running = run_handler_with_pushes(&mut ctx, call);
        tokio::pin!(running);
        assert!(
            futures::poll!(running.as_mut()).is_pending(),
            "the write must still be waiting for the peer"
        );
        assert!(
            finished.load(Ordering::SeqCst),
            "the handler must be polled while the push write waits"
        );

        release
            .send(())
            .expect("the handler run still holds the release channel");
        running.await.expect("the release lets the write finish")
    };

    assert_eq!(
        response,
        Some(HANDLER_RESPONSE),
        "the response produced during the write must be returned"
    );
    assert_eq!(
        sink.written.len(),
        1,
        "the queued push must reach the transport"
    );
    assert_eq!(
        sink.written.first().map(|frame| frame.tag),
        Some(7),
        "the frame that reached the transport must be the queued push"
    );
}

/// An abandoned push write stops the connection, even with a finished handler.
#[tokio::test]
async fn an_abandoned_write_discards_the_finished_response() {
    let shutdown = CancellationToken::new();
    let (mut actor, handle) = build_actor(shutdown.clone()).expect("actor queues build");
    handle
        .push_high_priority(queued_push())
        .await
        .expect("the lane has capacity");
    let (_release, blocked) = oneshot::channel();
    let mut sink = BlockingSink::new(blocked);
    let codec = TestCodec::new(MAX_FRAME_LENGTH);
    let serializer = BincodeSerializer;
    let mut pipeline: FramePipeline<TestFrame> = FramePipeline::new(None);
    let mut ctx = ResponseContext {
        serializer: &serializer,
        sink: &mut sink,
        pipeline: &mut pipeline,
        codec: &codec,
        actor: &mut actor,
        protocol: None,
    };
    let finished = Arc::new(AtomicBool::new(false));
    let call = DeferredHandler::new(Arc::clone(&finished));

    let response = {
        let running = run_handler_with_pushes(&mut ctx, call);
        tokio::pin!(running);
        assert!(
            futures::poll!(running.as_mut()).is_pending(),
            "the write must be pending before shutdown is requested"
        );
        shutdown.cancel();
        running.await.expect("an abandoned write is not an error")
    };

    assert_eq!(
        response, None,
        "a response behind an abandoned write must not be returned"
    );
    assert!(
        sink.written.is_empty(),
        "the abandoned write must not reach the transport"
    );
}

/// Sink that records every frame and requests shutdown after the first one.
///
/// A writable transport models the reported case: shutdown arrives while the
/// writes themselves always succeed, so only the loop's own shutdown
/// observation can end the run.
struct StopAfterFirstWrite {
    /// Token cancelled once the first frame has been accepted, when set.
    shutdown: Option<CancellationToken>,
    /// Frames the transport accepted, in write order.
    written: Vec<TestFrame>,
}

impl Sink<TestFrame> for StopAfterFirstWrite {
    type Error = io::Error;

    fn poll_ready(self: Pin<&mut Self>, _cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        Poll::Ready(Ok(()))
    }

    fn start_send(mut self: Pin<&mut Self>, item: TestFrame) -> Result<(), Self::Error> {
        self.written.push(item);
        if let Some(shutdown) = self.shutdown.take() {
            shutdown.cancel();
        }
        Ok(())
    }

    fn poll_flush(self: Pin<&mut Self>, _cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        Poll::Ready(Ok(()))
    }

    fn poll_close(self: Pin<&mut Self>, _cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        Poll::Ready(Ok(()))
    }
}

/// A handler kept alive by its own pushes must still observe shutdown.
///
/// The sink stays writable, so every push write completes before the write's
/// own shutdown arm could run. Without observing shutdown between batches the
/// run would never end, and the connection could never finish draining.
#[tokio::test]
async fn shutdown_between_push_batches_abandons_the_handler() {
    let shutdown = CancellationToken::new();
    let (mut actor, handle) = build_actor(shutdown.clone()).expect("actor queues build");
    let mut sink = StopAfterFirstWrite {
        shutdown: Some(shutdown),
        written: Vec::new(),
    };
    let codec = TestCodec::new(MAX_FRAME_LENGTH);
    let serializer = BincodeSerializer;
    let mut pipeline: FramePipeline<TestFrame> = FramePipeline::new(None);
    let mut ctx = ResponseContext {
        serializer: &serializer,
        sink: &mut sink,
        pipeline: &mut pipeline,
        codec: &codec,
        actor: &mut actor,
        protocol: None,
    };
    let producer = handle.clone();
    // The handler pushes once and then never finishes, so only a shutdown that
    // is observed between batches can end the run.
    let call = async move {
        producer
            .push_high_priority(queued_push())
            .await
            .expect("the lane has capacity");
        std::future::pending::<()>().await;
    };

    let response = {
        let running = run_handler_with_pushes(&mut ctx, call);
        tokio::time::timeout(std::time::Duration::from_secs(5), running)
            .await
            .expect("shutdown between batches must end the run")
            .expect("an abandoned run is not an error")
    };

    assert_eq!(
        response, None,
        "an abandoned handler must not deliver a response"
    );
    assert_eq!(
        sink.written.len(),
        1,
        "the frame written before the stop reached the transport"
    );
    assert_eq!(
        handle.push_high_priority(queued_push()).await,
        Err(crate::push::PushError::Closed),
        "the abandoned run must stop accepting pushes"
    );
}
