//! Completion-hook ordering on the prepared response path.
//!
//! `forward_response` runs the protocol's `on_command_end` itself, because the
//! actor built for a prepared connection carries no completion hook. These
//! tests pin that contract: the hook runs once after a successful terminator
//! write, and never when the write fails or the response is cancelled.

use std::{
    convert::Infallible,
    io,
    pin::Pin,
    sync::{Arc, Mutex, PoisonError},
    task::{Context, Poll},
    time::Duration,
};

use bytes::Bytes;
use futures::{Sink, StreamExt};
use tokio_util::sync::CancellationToken;

use super::{Dispatch, ResponseContext};
use crate::{
    app::{Envelope, codec_driver::FramePipeline},
    codec::LengthDelimitedFrameCodec,
    connection::{ConnectionActor, ConnectionChannels},
    hooks::{ConnectionContext, ProtocolHooks, WireframeProtocol},
    middleware::{HandlerService, Service, ServiceRequest, ServiceResponse},
    push::{PushConfigError, PushHandle, PushQueues},
    response::{FrameStream, Response, WireframeError},
    serializer::BincodeSerializer,
};

/// Ordered record of the writes and hooks observed by one scenario.
type EventLog = Arc<Mutex<Vec<&'static str>>>;

/// Append one event to the shared log.
fn record(log: &EventLog, event: &'static str) {
    log.lock()
        .unwrap_or_else(PoisonError::into_inner)
        .push(event);
}

/// Snapshot the events recorded so far.
fn events(log: &EventLog) -> Vec<&'static str> {
    log.lock().unwrap_or_else(PoisonError::into_inner).clone()
}

/// Protocol that records its completion hook and supplies a terminator.
struct RecordingProtocol {
    /// Shared log the hook appends to.
    log: EventLog,
}

impl WireframeProtocol for RecordingProtocol {
    type Frame = Bytes;
    type ProtocolError = ();

    fn stream_end_frame(&self, _ctx: &mut ConnectionContext) -> Option<Self::Frame> {
        Some(Bytes::from_static(b"END"))
    }

    fn on_command_end(&self, _ctx: &mut ConnectionContext) { record(&self.log, "command_end"); }
}

/// Sink that records each write and can refuse the first one.
struct RecordingSink {
    /// Shared log each accepted write appends to.
    log: EventLog,
    /// Whether every write is refused.
    fail: bool,
}

impl Sink<Bytes> for RecordingSink {
    type Error = io::Error;

    fn poll_ready(self: Pin<&mut Self>, _cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        Poll::Ready(Ok(()))
    }

    fn start_send(self: Pin<&mut Self>, _item: Bytes) -> Result<(), Self::Error> {
        if self.fail {
            return Err(io::Error::other("sink refused the write"));
        }
        record(&self.log, "send");
        Ok(())
    }

    fn poll_flush(self: Pin<&mut Self>, _cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        Poll::Ready(Ok(()))
    }

    fn poll_close(self: Pin<&mut Self>, _cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        Poll::Ready(Ok(()))
    }
}

/// Sink that never accepts a write, modelling a peer that stopped reading.
struct StalledSink {
    /// Shared log, retained so the test can assert nothing was written.
    log: EventLog,
}

impl Sink<Bytes> for StalledSink {
    type Error = io::Error;

    fn poll_ready(self: Pin<&mut Self>, _cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        record(&self.log, "poll_ready");
        Poll::Pending
    }

    fn start_send(self: Pin<&mut Self>, _item: Bytes) -> Result<(), Self::Error> {
        record(&self.log, "send");
        Ok(())
    }

    fn poll_flush(self: Pin<&mut Self>, _cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        Poll::Ready(Ok(()))
    }

    fn poll_close(self: Pin<&mut Self>, _cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        Poll::Ready(Ok(()))
    }
}

/// Service that hands the prepared path a pre-built response.
struct StreamService {
    /// Deferred slot the response is installed in.
    deferred: Arc<Mutex<Option<Response<Envelope, ()>>>>,
}

#[async_trait::async_trait]
impl Service for StreamService {
    type Error = Infallible;

    async fn call(&self, _req: ServiceRequest) -> Result<ServiceResponse, Self::Error> {
        Ok(ServiceResponse::with_deferred(
            Vec::new(),
            None,
            Arc::clone(&self.deferred),
        ))
    }
}

/// Build an untracked actor whose push queues stay open for the test.
///
/// # Errors
///
/// Returns [`PushConfigError`] if the queue capacities are rejected.
fn build_actor() -> Result<(ConnectionActor<Envelope, ()>, PushHandle<Envelope>), PushConfigError> {
    build_actor_with(CancellationToken::new())
}

/// Build an actor whose shutdown token the test controls.
///
/// # Errors
///
/// Returns [`PushConfigError`] if the queue capacities are rejected.
fn build_actor_with(
    shutdown: CancellationToken,
) -> Result<(ConnectionActor<Envelope, ()>, PushHandle<Envelope>), PushConfigError> {
    let (queues, handle) = PushQueues::<Envelope>::builder()
        .high_capacity(1)
        .low_capacity(1)
        .build()?;
    let actor = ConnectionActor::new_untracked(
        ConnectionChannels::new(queues, handle.clone()),
        None,
        shutdown,
        ProtocolHooks::default(),
    );
    Ok((actor, handle))
}

/// Run one response through the prepared path and return its event log.
///
/// `fail_sink` refuses every write, and `cancel_peer` marks the peer
/// unobservable with a zero grace so the response is cancelled immediately.
///
/// # Errors
///
/// Returns the push-queue configuration failure. The dispatch decision is
/// returned alongside the log so a failing scenario can still inspect it.
async fn forward(
    response: Response<Envelope, ()>,
    fail_sink: bool,
    cancel_peer: bool,
) -> Result<(EventLog, io::Result<Dispatch>), PushConfigError> {
    let log: EventLog = Arc::new(Mutex::new(Vec::new()));
    let (mut actor, _handle) = build_actor()?;
    if cancel_peer {
        let peer_closed = CancellationToken::new();
        peer_closed.cancel();
        actor.set_peer_closed(peer_closed);
        actor.set_peer_closed_grace(Duration::ZERO);
    }
    let mut sink = RecordingSink {
        log: Arc::clone(&log),
        fail: fail_sink,
    };
    let outcome = run_forward(&mut sink, &mut actor, response, &log).await;
    Ok((log, outcome))
}

/// Run one response through the prepared path with an already built actor.
async fn run_forward<K>(
    sink: &mut K,
    actor: &mut ConnectionActor<Envelope, ()>,
    response: Response<Envelope, ()>,
    log: &EventLog,
) -> io::Result<Dispatch>
where
    K: Sink<Bytes, Error = io::Error> + Unpin + Send,
{
    let codec = LengthDelimitedFrameCodec::new(1024);
    let serializer = BincodeSerializer;
    let mut pipeline: FramePipeline<Bytes> =
        FramePipeline::with_hooks(None, ProtocolHooks::default());
    let service = HandlerService::<Envelope>::from_service(
        7,
        StreamService {
            deferred: Arc::new(Mutex::new(Some(response))),
        },
    );
    let protocol: Arc<dyn WireframeProtocol<Frame = Bytes, ProtocolError = ()>> =
        Arc::new(RecordingProtocol {
            log: Arc::clone(log),
        });

    super::forward_response(
        Envelope::new(7, Some(3), Vec::new()),
        &service,
        ResponseContext {
            serializer: &serializer,
            sink,
            pipeline: &mut pipeline,
            codec: &codec,
            actor,
            protocol: Some(&protocol),
        },
    )
    .await
}

/// A completed stream runs the completion hook once, after the terminator.
#[tokio::test]
async fn completion_hook_runs_once_after_the_terminator_write() {
    let stream: FrameStream<Envelope, ()> = futures::stream::empty().boxed();
    let (log, outcome) = forward(Response::Stream(stream), false, false)
        .await
        .expect("the actor should build");
    assert_eq!(
        outcome.expect("a completed response is not an error"),
        Dispatch::Continue,
        "a completed response lets the loop dispatch the next frame"
    );

    assert_eq!(
        events(&log),
        vec!["send", "command_end"],
        "the hook must run once, after the terminator reached the sink"
    );
}

/// A failed terminator write suppresses the completion hook.
#[tokio::test]
async fn completion_hook_is_suppressed_when_the_terminator_write_fails() {
    let stream: FrameStream<Envelope, ()> = futures::stream::empty().boxed();
    let (log, outcome) = forward(Response::Stream(stream), true, false)
        .await
        .expect("the actor should build");

    let error = outcome.expect_err("a refused write must fail the response");
    assert!(
        error.to_string().contains("sink refused the write"),
        "the transport error must be propagated: {error}"
    );
    assert!(
        events(&log).is_empty(),
        "no hook may run when the terminator was never written"
    );
}

/// A stream that reports a protocol error stops dispatch without a terminator.
///
/// The failed stream ends without a terminator, so a client cannot tell where
/// it ended. Serving the request buffered behind it would make that response
/// read as part of the failed stream.
#[tokio::test]
async fn protocol_error_stops_dispatch_without_a_terminator() {
    let failed: FrameStream<Envelope, ()> = Box::pin(futures::stream::iter(vec![
        Ok(Envelope::new(7, Some(3), vec![1])),
        Err(WireframeError::Protocol(())),
    ]));
    let (log, outcome) = forward(Response::Stream(failed), false, false)
        .await
        .expect("the actor should build");
    assert_eq!(
        outcome.expect("a protocol error is not a transport error"),
        Dispatch::Stop,
        "a failed stream must stop the loop before it serves another frame"
    );

    assert_eq!(
        events(&log),
        vec!["send", "command_end"],
        "exactly one frame is written and no terminator follows the failure"
    );
}

/// A terminator the peer never accepts is abandoned on shutdown.
///
/// The terminator is the last frame the prepared path writes. A peer that
/// stopped reading applies back-pressure that can hold that write open
/// indefinitely, which would pin a graceful server drain after the response
/// itself completed. The shutdown token must therefore bound it, and the
/// abandoned terminator must suppress the completion hook.
#[tokio::test]
async fn shutdown_abandons_a_terminator_the_peer_never_accepts() {
    let log: EventLog = Arc::new(Mutex::new(Vec::new()));
    let shutdown = CancellationToken::new();
    let (mut actor, _handle) = build_actor_with(shutdown.clone()).expect("the actor should build");
    let mut sink = StalledSink {
        log: Arc::clone(&log),
    };

    // An empty stream writes only the protocol terminator, so a pending drive
    // proves the terminator itself is what stalled.
    let stream: FrameStream<Envelope, ()> = futures::stream::empty().boxed();
    let outcome = {
        let mut driving = std::pin::pin!(run_forward(
            &mut sink,
            &mut actor,
            Response::Stream(stream),
            &log
        ));
        assert!(
            futures::poll!(driving.as_mut()).is_pending(),
            "the terminator write must stall before shutdown is requested"
        );

        shutdown.cancel();
        driving
            .await
            .expect("an abandoned terminator is not a transport error")
    };

    assert_eq!(
        outcome,
        Dispatch::Stop,
        "a cancelled terminator must stop the loop before it serves another frame"
    );
    let observed = events(&log);
    assert!(
        !observed.contains(&"send"),
        "the abandoned terminator must never reach the sink: {observed:?}"
    );
    assert!(
        !observed.contains(&"command_end"),
        "a cancelled terminator must not run the completion hook: {observed:?}"
    );
}

/// A cancelled response suppresses the completion hook.
#[tokio::test]
async fn completion_hook_is_suppressed_when_the_response_is_cancelled() {
    let stalled: FrameStream<Envelope, ()> = Box::pin(futures::stream::pending::<
        Result<Envelope, WireframeError<()>>,
    >());
    let (log, outcome) = forward(Response::Stream(stalled), false, true)
        .await
        .expect("the actor should build");
    assert_eq!(
        outcome.expect("a cancelled response is not an error"),
        Dispatch::Stop,
        "a cancelled stream must stop the loop before it serves another frame"
    );

    assert!(
        events(&log).is_empty(),
        "a cancelled response writes no terminator and runs no hook"
    );
}
