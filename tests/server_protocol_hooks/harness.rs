//! Shared harness for the server protocol hook tests.
//!
//! One application builder, one server launcher, and one set of frame helpers
//! keep every scenario in this test binary observing the same wire format. The
//! teardown signal is the synchronization point: teardown runs after stream
//! processing stops, so a scenario that awaits it may assert on the complete
//! sequence of hook events without a wall-clock pause.

use std::{
    net::SocketAddr,
    sync::{Arc, Mutex, PoisonError},
    time::Duration,
};

use bytes::Bytes;
use futures::StreamExt;
use rstest::fixture;
use tokio::{io::AsyncWriteExt, net::TcpStream, sync::oneshot, time::timeout};
use tokio_util::codec::Framed;
use wireframe::{
    WireframeError,
    app::{Envelope, Packet, ResponseHandler, WireframeApp},
    codec::{FrameCodec, LengthDelimitedFrameCodec},
    frame::FrameMetadata,
    prelude::{DecodeWith, EncodeWith},
    response::{FrameStream, Response},
    serializer::{BincodeSerializer, MessageCompatibilitySerializer, Serializer},
    server::{ServerShutdown, WireframeServer},
};
use wireframe_testing::{
    TestError,
    TestResult,
    encode_frame,
    unused_listener,
    wait_for_server_readiness,
};

use crate::hook_recorder::{HookLog, MARKER, RecordingProtocol};

/// Application type shared by the request builder and the server under test.
pub(super) type TestApp = WireframeApp<BincodeSerializer, (), Envelope>;

/// Route every scenario requests.
pub(super) const ROUTE: u32 = 79;
/// Frame the recording protocol returns from `stream_end_frame`.
pub(super) const TERMINATOR: &[u8] = b"END";
/// Maximum frame length used by the client's decoder.
pub(super) const MAX_FRAME_LENGTH: usize = 2048;
/// Upper bound for one frame and for the connection's teardown.
pub(super) const STEP_TIMEOUT: Duration = Duration::from_secs(20);
/// Malformed frames the server tolerates before it closes the connection.
pub(super) const DESER_FAILURE_LIMIT: usize = 10;

/// Require `condition`, reporting `message` as the test failure otherwise.
pub(super) fn ensure(condition: bool, message: impl Into<String>) -> TestResult {
    if condition {
        Ok(())
    } else {
        Err(TestError::Msg(message.into()))
    }
}

/// Build the teardown closure that signals when a connection ends.
///
/// Teardown runs after stream processing stops, so awaiting the signal proves
/// the connection loop has finished before a test asserts on its hook events.
/// The sender is taken out of its slot, so a second teardown on the same
/// connection cannot send twice.
pub(super) fn ended_notifier(
    ended: oneshot::Sender<()>,
) -> impl Fn(()) -> std::future::Ready<()> + Send + Sync + 'static {
    let slot = Arc::new(Mutex::new(Some(ended)));
    move |()| {
        let maybe_sender = slot.lock().unwrap_or_else(PoisonError::into_inner).take();
        if let Some(sender) = maybe_sender {
            let _sent = sender.send(());
        }
        std::future::ready(())
    }
}

/// Build the application with the recording protocol and a teardown signal.
pub(super) fn build_app(
    log: &Arc<HookLog>,
    terminator: Option<Bytes>,
    ended: oneshot::Sender<()>,
    handler: ResponseHandler<Envelope>,
) -> TestResult<TestApp> {
    Ok(TestApp::default()
        .route_response(ROUTE, handler)?
        .with_protocol(RecordingProtocol::new(Arc::clone(log), terminator))
        .on_connection_setup(|| async {})?
        .on_connection_teardown(ended_notifier(ended))?)
}

/// A server bound to a test listener, ready to accept one connection.
pub(super) struct RunningServer {
    /// Address the client connects to.
    pub(super) addr: SocketAddr,
    /// Handle that stops the server and waits for its drain.
    shutdown: ServerShutdown,
}

impl RunningServer {
    /// Stop the server and wait for every connection task to finish.
    pub(super) async fn stop(self) -> TestResult {
        self.shutdown.stop();
        timeout(STEP_TIMEOUT, self.shutdown.drained())
            .await
            .map_err(|_| TestError::Msg("the server did not drain".into()))??;
        Ok(())
    }
}

/// Spawn a one-worker server for `app`.
///
/// Generic over the application's codec so a scenario can install its own
/// framing while every other detail of the launch stays identical.
pub(super) async fn start_server<Ser, Ctx, E, Codec>(
    app: WireframeApp<Ser, Ctx, E, Codec>,
) -> TestResult<RunningServer>
where
    Ser: Serializer
        + FrameMetadata<Frame = Envelope>
        + MessageCompatibilitySerializer
        + Send
        + Sync
        + 'static,
    Ctx: Send + 'static,
    E: Packet,
    Codec: FrameCodec,
    Envelope: DecodeWith<Ser> + EncodeWith<Ser>,
{
    let listener = unused_listener()?;
    let server = WireframeServer::from_app(app)
        .workers(1)
        .bind_existing_listener(listener)?;
    let addr = server
        .local_addr()
        .ok_or_else(|| TestError::Msg("server local address missing".into()))?;
    let (ready_tx, ready_rx) = oneshot::channel();
    let shutdown = server.ready_signal(ready_tx).spawn().await?;
    wait_for_server_readiness(ready_rx).await?;
    Ok(RunningServer { addr, shutdown })
}

/// Wait until the connection under test has stopped processing.
pub(super) async fn await_connection_end(ended: oneshot::Receiver<()>) -> TestResult {
    timeout(STEP_TIMEOUT, ended)
        .await
        .map_err(|_| TestError::Msg("the connection never ended".into()))?
        .map_err(|_| TestError::Msg("the connection teardown signal was dropped".into()))
}

/// One running server, its hook log, and the signal that it wound down.
///
/// The three travel together because a scenario is only meaningful with all of
/// them: the log proves what ran, and the teardown signal is what makes those
/// events complete before the test asserts on them.
pub(super) struct HookScenario {
    /// Hook events recorded for the connection.
    pub(super) log: Arc<HookLog>,
    /// Address the client connects to.
    pub(super) addr: SocketAddr,
    /// Signal that connection teardown finished, consumed by [`Self::await_ended`].
    ended: Option<oneshot::Receiver<()>>,
    /// Handle that stops the server at the end of the scenario.
    server: RunningServer,
}

impl HookScenario {
    /// Open a client connection and send the standard request.
    pub(super) async fn connect_request(
        &self,
    ) -> TestResult<impl futures::Stream<Item = std::io::Result<Bytes>> + Unpin> {
        let mut client = self.connect_raw().await?;
        client.write_all(&build_request(ROUTE)?).await?;
        Ok(Framed::new(client, client_codec().decoder()))
    }

    /// Open a client connection whose bytes the test writes itself.
    pub(super) async fn connect_raw(&self) -> TestResult<TcpStream> {
        Ok(TcpStream::connect(self.addr).await?)
    }

    /// Wait until the connection under test has stopped processing.
    ///
    /// Taking the receiver through `&mut self` keeps the scenario whole, so the
    /// server can still be stopped afterwards.
    pub(super) async fn await_ended(&mut self) -> TestResult {
        let ended = self
            .ended
            .take()
            .ok_or_else(|| TestError::Msg("teardown was already awaited".into()))?;
        await_connection_end(ended).await
    }

    /// Stop the server and wait for every connection task to finish.
    pub(super) async fn stop(self) -> TestResult { self.server.stop().await }
}

/// Fixture: one server whose protocol records every hook event.
///
/// The terminator and response handler decide which hooks the scenario
/// exercises; every other hook observation is the same across scenarios.
#[fixture]
pub(super) async fn scenario(
    #[default(None)] terminator: Option<Bytes>,
    #[default(single_handler())] handler: ResponseHandler<Envelope>,
) -> TestResult<HookScenario> {
    let log = Arc::new(HookLog::default());
    let (ended_tx, ended) = oneshot::channel();
    let app = build_app(&log, terminator, ended_tx, handler)?;
    let server = start_server(app).await?;
    Ok(HookScenario {
        log,
        addr: server.addr,
        ended: Some(ended),
        server,
    })
}

/// Encoder for frames the client writes.
pub(super) fn client_codec() -> LengthDelimitedFrameCodec {
    LengthDelimitedFrameCodec::new(MAX_FRAME_LENGTH)
}

/// Encode an empty request for `route` in the transport's frame format.
pub(super) fn build_request(route: u32) -> TestResult<Vec<u8>> {
    let encoded = BincodeSerializer.serialize(&Envelope::new(route, Some(7), Vec::new()))?;
    let mut codec = TestApp::default().length_codec();
    Ok(encode_frame(&mut codec, encoded)?)
}

/// Encode one raw payload as a complete codec frame.
pub(super) fn encode_payload(payload: &[u8]) -> TestResult<Vec<u8>> {
    let mut codec = TestApp::default().length_codec();
    Ok(encode_frame(&mut codec, payload.to_vec())?)
}

/// Read the next frame, failing the test if none arrives in time.
pub(super) async fn next_frame<S>(frames: &mut S) -> TestResult<Bytes>
where
    S: futures::Stream<Item = Result<Bytes, std::io::Error>> + Unpin,
{
    timeout(STEP_TIMEOUT, frames.next())
        .await
        .map_err(|_| TestError::Msg("the server stopped sending frames".into()))?
        .transpose()
        .map_err(TestError::from)?
        .ok_or_else(|| TestError::Msg("the connection ended early".into()))
}

/// Strip the marker the protocol appends to every outbound frame.
pub(super) fn marked_body(frame: &Bytes) -> TestResult<Vec<u8>> {
    let split = frame
        .len()
        .checked_sub(1)
        .ok_or_else(|| TestError::Msg("an outbound frame was empty".into()))?;
    let (body, marker) = frame.split_at(split);
    ensure(
        marker == [MARKER],
        format!("the frame was not marked by before_send: {frame:?}"),
    )?;
    Ok(body.to_vec())
}

/// Decode one application frame after removing the protocol marker.
pub(super) fn decode_envelope(frame: &Bytes) -> TestResult<Envelope> {
    decode_unmarked_envelope(&marked_body(frame)?)
}

/// Decode an application payload that carries no protocol marker.
///
/// [`WireframeProtocol::before_send`] records the frame it observed *before*
/// mutating it, so a frame taken from a recorded event has no marker to strip.
///
/// [`WireframeProtocol::before_send`]: wireframe::hooks::WireframeProtocol::before_send
pub(super) fn decode_unmarked_envelope(body: &[u8]) -> TestResult<Envelope> {
    let (envelope, consumed) = BincodeSerializer.deserialize::<Envelope>(body)?;
    ensure(
        consumed == body.len(),
        format!(
            "trailing bytes after envelope: {consumed} of {}",
            body.len()
        ),
    )?;
    Ok(envelope)
}

/// Handler that streams two frames, so the response owes a terminator.
pub(super) fn stream_handler() -> ResponseHandler<Envelope> {
    Arc::new(|_request: &Envelope| {
        Box::pin(async {
            let stream: FrameStream<Envelope, ()> = Box::pin(futures::stream::iter(vec![
                Ok(Envelope::new(ROUTE, None, vec![1])),
                Ok(Envelope::new(ROUTE, None, vec![2])),
            ]));
            Response::Stream(stream)
        })
    })
}

/// Handler whose response fails with a protocol error.
///
/// The error travels through the response stream because that is the only
/// path that reaches [`WireframeProtocol::handle_error`]; a handler that
/// returns `Err` is logged and produces no protocol callback.
///
/// [`WireframeProtocol::handle_error`]: wireframe::hooks::WireframeProtocol::handle_error
pub(super) fn failing_handler() -> ResponseHandler<Envelope> {
    Arc::new(|_request: &Envelope| {
        Box::pin(async {
            let stream: FrameStream<Envelope, ()> = Box::pin(futures::stream::iter(vec![Err(
                WireframeError::Protocol(()),
            )]));
            Response::Stream(stream)
        })
    })
}

/// Handler that answers with one complete response frame.
pub(super) fn single_handler() -> ResponseHandler<Envelope> {
    Arc::new(|_request: &Envelope| {
        Box::pin(async { Response::Single(Envelope::new(ROUTE, None, vec![9])) })
    })
}

/// Handler that pushes two frames and then answers.
///
/// The push queue holds one frame per priority, so the second push only
/// completes if the connection keeps draining pushes while the handler is
/// pending. A connection that waited for the handler first would deadlock.
pub(super) fn push_then_reply(log: Arc<HookLog>) -> ResponseHandler<Envelope> {
    Arc::new(move |_request: &Envelope| {
        let hook_log = Arc::clone(&log);
        Box::pin(async move {
            let handle = hook_log
                .push_handle()
                .expect("connection setup must capture the push handle");
            handle
                .push_high_priority(Bytes::from_static(b"PUSH-1"))
                .await
                .expect("the first push must be accepted");
            handle
                .push_high_priority(Bytes::from_static(b"PUSH-2"))
                .await
                .expect("the second push must be accepted");
            Response::Single(Envelope::new(ROUTE, None, vec![9]))
        })
    })
}
