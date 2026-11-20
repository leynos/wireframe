//! The handler keeps being polled while a push write waits for the transport.
//!
//! A peer that stops reading without closing leaves a push write pending while
//! the peer is still observed, so neither the server stop token nor the idle
//! grace bounds it. This module proves the handler is polled during that wait,
//! that a response it produces is returned once the write completes, and that
//! an abandoned write still stops the connection.

use std::{
    error::Error,
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
    push::{PushConfigError, PushError, PushHandle, PushPolicy, PushPriority, PushQueues},
    serializer::BincodeSerializer,
    test_helpers::frame_codec::{TestCodec, TestFrame},
};

/// Maximum frame length accepted by the test codec.
const MAX_FRAME_LENGTH: usize = 1024;

#[cfg(feature = "metrics")]
#[path = "response_pushes_tests/idle_grace.rs"]
mod idle_grace;

#[path = "response_pushes_tests/shutdown.rs"]
mod shutdown;

/// Response the deferred handler returns once it is polled twice.
const HANDLER_RESPONSE: &str = "handler finished";

/// Owned state that every response-push scenario drives.
///
/// The rig owns each value [`ResponseContext`] borrows, so a test can queue a
/// frame before the run and inspect what the sink received after it. The sink
/// is a type parameter because how the transport behaves is the one thing each
/// scenario varies.
struct HandlerRig<S> {
    /// Actor whose push lanes and shutdown token the run observes.
    actor: ConnectionActor<Envelope, (), TestFrame>,
    /// Handle that queues pushes for the test or the handler.
    handle: PushHandle<TestFrame>,
    /// Transport the run writes to.
    sink: S,
    /// Codec that wraps serialized responses in frames.
    codec: TestCodec,
    /// Serializer for the handler's response value.
    serializer: BincodeSerializer,
    /// Outbound pipeline applied to every frame.
    pipeline: FramePipeline<TestFrame>,
    /// Token whose cancellation asks the actor to stop.
    shutdown: CancellationToken,
}

impl<S> HandlerRig<S>
where
    S: Sink<TestFrame, Error = io::Error> + Send + Unpin,
{
    /// Build a rig for `sink`, with an actor observing `shutdown`.
    ///
    /// # Errors
    ///
    /// Returns [`PushConfigError`] if the queue capacities are rejected.
    fn new(shutdown: CancellationToken, sink: S) -> Result<Self, PushConfigError> {
        let (queues, handle) = PushQueues::<TestFrame>::builder()
            .high_capacity(1)
            .low_capacity(1)
            .build()?;
        let actor = ConnectionActor::new_untracked(UntrackedActorInit {
            channels: ConnectionChannels::new(queues, handle.clone()),
            response: None,
            shutdown: shutdown.clone(),
            hooks: ProtocolHooks::default(),
            ctx: ConnectionContext::default(),
        });
        Ok(Self {
            actor,
            handle,
            sink,
            codec: TestCodec::new(MAX_FRAME_LENGTH),
            serializer: BincodeSerializer,
            pipeline: FramePipeline::new(None),
            shutdown,
        })
    }

    /// Queue one push on the high-priority lane without waiting for it.
    ///
    /// # Errors
    ///
    /// Returns the refusal from [`PushHandle::try_push`], which reports a full
    /// or closed lane rather than dropping the frame silently.
    fn try_queue_push(&mut self) -> Result<(), PushError> {
        self.handle.try_push(
            queued_push(),
            PushPriority::High,
            PushPolicy::ReturnErrorIfFull,
        )
    }

    /// Borrow the rig's fields as the context a handler run takes.
    const fn context(&mut self) -> ResponseContext<'_, BincodeSerializer, S, TestCodec> {
        ResponseContext {
            serializer: &self.serializer,
            sink: &mut self.sink,
            pipeline: &mut self.pipeline,
            codec: &self.codec,
            actor: &mut self.actor,
        }
    }
}

/// The blocked rig and the sender that releases its pending write.
type BlockedRig = (HandlerRig<BlockingSink>, oneshot::Sender<()>);

/// Fallible payload of [`blocked_rig`].
///
/// A test unwraps it in its own body, where a failure is the test verdict.
type BlockedRigFixture = Result<BlockedRig, Box<dyn Error>>;

/// Rig whose sink accepts nothing until the returned sender fires.
///
/// One push is already queued, so every scenario starts from the same frame;
/// a test that queues more frames does so from the handler.
#[rstest::fixture]
fn blocked_rig() -> BlockedRigFixture {
    let (release, blocked) = oneshot::channel();
    let mut rig = HandlerRig::new(CancellationToken::new(), BlockingSink::new(blocked))?;
    rig.try_queue_push()?;
    Ok((rig, release))
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
#[rstest::rstest]
#[tokio::test]
async fn the_handler_keeps_running_while_a_push_write_waits(blocked_rig: BlockedRigFixture) {
    let (mut rig, release) = blocked_rig.expect("the rig's queues build");
    let finished = Arc::new(AtomicBool::new(false));
    let call = DeferredHandler::new(Arc::clone(&finished));

    let response = {
        let mut ctx = rig.context();
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
        rig.sink.written.len(),
        1,
        "the queued push must reach the transport"
    );
    assert_eq!(
        rig.sink.written.first().map(|frame| frame.tag),
        Some(7),
        "the frame that reached the transport must be the queued push"
    );
}

/// An abandoned push write stops the connection, even with a finished handler.
#[rstest::rstest]
#[tokio::test]
async fn an_abandoned_write_discards_the_finished_response(blocked_rig: BlockedRigFixture) {
    let (mut rig, _release) = blocked_rig.expect("the rig's queues build");
    // Cloned before the run borrows the rig, so the stop request below needs no
    // access to it while the run is alive.
    let shutdown = rig.shutdown.clone();
    let finished = Arc::new(AtomicBool::new(false));
    let call = DeferredHandler::new(Arc::clone(&finished));

    let response = {
        let mut ctx = rig.context();
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
        rig.sink.written.is_empty(),
        "the abandoned write must not reach the transport"
    );
}
