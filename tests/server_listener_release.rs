//! The listener must be released when the accept loops exit.
//!
//! A graceful drain keeps in-flight connection tasks running after shutdown is
//! requested, but the socket itself has no further work once no accept loop is
//! accepting on it. Holding a listener handle past that point keeps the port
//! bound for as long as a handler takes, so the address cannot be handed off
//! and a caller can only discover the release by polling.
//!
//! Each test here parks one request inside its handler, requests shutdown, and
//! requires a refused connection before the handler is released. They fail
//! without the supervisor dropping its own listener handle once the accept
//! loops hold their clones.
#![cfg(not(loom))]
#![forbid(unsafe_code)]

use std::{io, net::SocketAddr, sync::Arc, time::Duration};

use futures::StreamExt;
use tokio::{
    io::AsyncWriteExt,
    net::TcpStream,
    sync::{Notify, oneshot},
    task::JoinHandle,
    time::{sleep, timeout},
};
use tokio_util::codec::Framed;
use wireframe::{
    app::{Envelope, ResponseHandler, WireframeApp},
    codec::{FrameCodec, LengthDelimitedFrameCodec},
    response::{FrameStream, Response},
    serializer::{BincodeSerializer, Serializer},
    server::{ServerError, ServerShutdown, WireframeServer},
};
use wireframe_testing::{
    TestError,
    TestResult,
    encode_frame,
    unused_listener,
    wait_for_server_readiness,
};

/// Application type shared by the request builder and the server under test.
type TestApp = WireframeApp<BincodeSerializer, (), Envelope>;
/// Client stream type for the length-delimited transport used here.
type HeldClient = Framed<TcpStream, <LengthDelimitedFrameCodec as FrameCodec>::Decoder>;

/// Route whose handler parks until the test releases it.
const HELD_ROUTE: u32 = 62;
/// Payload the parked handler answers with.
const REPLY: u8 = 4;
/// Maximum frame length used by the transport.
const MAX_FRAME_LENGTH: usize = 2048;
/// Upper bound for the handler to report that it started.
const START_TIMEOUT: Duration = Duration::from_secs(5);
/// Upper bound for a probe connection attempt.
///
/// A probe against a listener that is still bound but unaccepted can sit in
/// the backlog, so the attempt is bounded rather than left to hang.
const PROBE_TIMEOUT: Duration = Duration::from_secs(1);
/// Upper bound for a response frame that must already be deliverable.
const FRAME_TIMEOUT: Duration = Duration::from_secs(5);
/// Window used to show that a drain has not resolved yet.
const NOT_YET: Duration = Duration::from_millis(100);

/// Report `message` as a test failure when `condition` does not hold.
fn ensure(condition: bool, message: impl Into<String>) -> TestResult {
    if condition {
        Ok(())
    } else {
        Err(TestError::Msg(message.into()))
    }
}

/// Handle a request with a stream that stalls after its first frame.
///
/// The response has provably started — the client reads the first frame — and
/// then parks on `release`, so the request is in flight in the sense the drain
/// cares about. A handler that produced nothing at all would be abandoned by
/// the stop instead, which is the documented trade-off rather than the
/// behaviour under test here.
fn held_handler(started: Arc<Notify>, release: Arc<Notify>) -> ResponseHandler<Envelope> {
    Arc::new(move |_request: &Envelope| {
        let handler_started = Arc::clone(&started);
        let handler_release = Arc::clone(&release);
        Box::pin(async move {
            let stream: FrameStream<Envelope, ()> = Box::pin(async_stream::try_stream! {
                yield Envelope::new(HELD_ROUTE, None, vec![REPLY]);
                handler_started.notify_one();
                handler_release.notified().await;
                yield Envelope::new(HELD_ROUTE, None, vec![REPLY + 1]);
            });
            Response::Stream(stream)
        })
    })
}

/// Build the application with the held route installed.
fn held_app(started: Arc<Notify>, release: Arc<Notify>) -> TestResult<TestApp> {
    Ok(TestApp::default().route_response(HELD_ROUTE, held_handler(started, release))?)
}

/// Encode an empty request for `route` in the transport's frame format.
fn build_request(route: u32) -> TestResult<Vec<u8>> {
    let encoded = BincodeSerializer.serialize(&Envelope::new(route, Some(7), Vec::new()))?;
    let mut codec = TestApp::default().length_codec();
    Ok(encode_frame(&mut codec, encoded)?)
}

/// Decode one response frame and return its payload.
fn decode_payload(bytes: &[u8]) -> TestResult<Vec<u8>> {
    let (envelope, consumed) = BincodeSerializer
        .deserialize::<Envelope>(bytes)
        .map_err(|error| TestError::Msg(format!("deserialize frame: {error}")))?;
    ensure(
        consumed == bytes.len(),
        format!(
            "trailing bytes after envelope: consumed {consumed} of {}",
            bytes.len()
        ),
    )?;
    Ok(envelope.payload_bytes().to_vec())
}

/// Read the next response frame, failing if none arrives in time.
async fn next_frame(client: &mut HeldClient) -> TestResult<Vec<u8>> {
    let frame = timeout(FRAME_TIMEOUT, client.next())
        .await
        .map_err(|_| TestError::Msg("the response stalled while the peer was reading".into()))?
        .transpose()
        .map_err(|error| TestError::Msg(format!("read response frame: {error}")))?
        .ok_or_else(|| TestError::Msg("the response ended before the request finished".into()))?;
    decode_payload(&frame)
}

/// Connect, send the held request, and read the frame that starts its response.
///
/// Reading the first frame is what proves the request reached its handler and
/// has a response in progress; the handler is parked after yielding it.
async fn park_request(addr: SocketAddr, started: &Arc<Notify>) -> TestResult<HeldClient> {
    let stream = TcpStream::connect(addr)
        .await
        .map_err(|error| TestError::Msg(format!("connect to {addr} failed: {error}")))?;
    let mut client = Framed::new(
        stream,
        LengthDelimitedFrameCodec::new(MAX_FRAME_LENGTH).decoder(),
    );
    client
        .get_mut()
        .write_all(&build_request(HELD_ROUTE)?)
        .await
        .map_err(|error| TestError::Msg(format!("sending the held request failed: {error}")))?;
    timeout(START_TIMEOUT, started.notified())
        .await
        .map_err(|_| TestError::Msg("the handler never started for the held request".into()))?;
    let first = next_frame(&mut client).await?;
    ensure(
        first == vec![REPLY],
        format!("the first response frame must precede the shutdown: {first:?}"),
    )?;
    Ok(client)
}

/// Probe whether `addr` still accepts a connection.
///
/// `Ok(true)` means the probe did not observe a refusal. Only `ECONNREFUSED`
/// counts as released: a reset does not establish that the address is unbound,
/// so it is treated as "not yet refused" and keeps the caller polling. Every
/// other outcome is an error, so a failed probe is never read as released.
async fn still_accepting(addr: SocketAddr) -> TestResult<bool> {
    match timeout(PROBE_TIMEOUT, TcpStream::connect(addr)).await {
        Ok(Ok(stream)) => {
            drop(stream);
            Ok(true)
        }
        Ok(Err(error)) if error.kind() == io::ErrorKind::ConnectionRefused => Ok(false),
        // A reset can be produced while the closing listener still has queued
        // state; it does not prove the address is unbound, so the caller keeps
        // polling until a refusal is observed.
        Ok(Err(error)) if error.kind() == io::ErrorKind::ConnectionReset => Ok(true),
        Ok(Err(error)) => Err(error.into()),
        // An attempt that never reached the socket says nothing either way.
        Err(_) => Err(TestError::Msg(format!(
            "probing {addr} did not complete within {PROBE_TIMEOUT:?}"
        ))),
    }
}

/// Wait until the address stops accepting while the request is still parked.
///
/// A shutdown request is asynchronous: the accept loops must be scheduled to
/// observe cancellation and exit before their clones release the socket. One
/// immediate probe would therefore race the release, so the assertion polls
/// under one bounded budget. The handler is gated for the whole wait, so a
/// successful refusal cannot be explained by the request having finished.
async fn assert_released_while_request_in_flight(addr: SocketAddr) -> TestResult {
    timeout(PROBE_TIMEOUT, async {
        loop {
            if !still_accepting(addr).await? {
                return Ok(());
            }
            sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .map_err(|_| {
        TestError::Msg(format!(
            "the listener at {addr} stayed bound for one second"
        ))
    })?
}

/// Release the parked handler and read the frame it then streams.
///
/// The refusal proves the socket is released. Receipt of this frame does not
/// by itself prove the response stream finished, and task or supervisor
/// termination is not observable here without a control to await, so the case
/// is recorded as observing the remaining frame only.
async fn finish_request(mut client: HeldClient, release: &Notify) -> TestResult {
    release.notify_one();
    let frame = next_frame(&mut client).await?;
    ensure(
        frame == vec![REPLY + 1],
        format!("the drained response must deliver its remaining frame: {frame:?}"),
    )
}

/// A dispatched stop releases the listener before the request finishes.
#[tokio::test]
async fn stop_releases_listener_before_the_request_finishes() -> TestResult {
    let started = Arc::new(Notify::new());
    let release = Arc::new(Notify::new());
    let app = held_app(Arc::clone(&started), Arc::clone(&release))?;
    let listener = unused_listener()?;
    let server = WireframeServer::from_app(app)
        .workers(1)
        .bind_existing_listener(listener)?;
    let addr = server.local_addr().ok_or("server local address missing")?;
    let (ready_tx, ready_rx) = oneshot::channel();
    let control: ServerShutdown = server.ready_signal(ready_tx).spawn().await?;
    wait_for_server_readiness(ready_rx).await?;

    let client = park_request(addr, &started).await?;
    control.stop();
    assert_released_while_request_in_flight(addr).await?;

    // The request is still parked, so a clean drain cannot have resolved yet.
    ensure(
        timeout(NOT_YET, control.drained()).await.is_err(),
        "the drain resolved before the in-flight request finished",
    )?;

    finish_request(client, &release).await?;
    timeout(NOT_YET, control.drained()).await.map_err(|_| {
        TestError::Msg("the drain did not complete after the request finished".into())
    })??;
    Ok(())
}

/// Dropping the last control releases the listener before the request finishes.
#[tokio::test]
async fn dropping_the_last_control_releases_listener_before_the_request_finishes() -> TestResult {
    let started = Arc::new(Notify::new());
    let release = Arc::new(Notify::new());
    let app = held_app(Arc::clone(&started), Arc::clone(&release))?;
    let listener = unused_listener()?;
    let server = WireframeServer::from_app(app)
        .workers(1)
        .bind_existing_listener(listener)?;
    let addr = server.local_addr().ok_or("server local address missing")?;
    let (ready_tx, ready_rx) = oneshot::channel();
    let control = server.ready_signal(ready_tx).spawn().await?;
    wait_for_server_readiness(ready_rx).await?;

    let client = park_request(addr, &started).await?;
    drop(control);
    assert_released_while_request_in_flight(addr).await?;
    finish_request(client, &release).await
}

/// Resolving `run_with_shutdown`'s future releases the listener first.
#[tokio::test]
async fn resolved_shutdown_future_releases_listener_before_the_request_finishes() -> TestResult {
    let started = Arc::new(Notify::new());
    let release = Arc::new(Notify::new());
    let app = held_app(Arc::clone(&started), Arc::clone(&release))?;
    let listener = unused_listener()?;
    let server = WireframeServer::from_app(app)
        .workers(1)
        .bind_existing_listener(listener)?;
    let addr = server.local_addr().ok_or("server local address missing")?;
    let (ready_tx, ready_rx) = oneshot::channel();
    let (shutdown_tx, shutdown_rx) = oneshot::channel();
    let task: JoinHandle<Result<(), ServerError>> =
        tokio::spawn(server.ready_signal(ready_tx).run_with_shutdown(async move {
            shutdown_rx.await.ok();
        }));
    wait_for_server_readiness(ready_rx).await?;

    let client = park_request(addr, &started).await?;
    shutdown_tx
        .send(())
        .map_err(|()| TestError::Msg("the shutdown future was already dropped".into()))?;
    assert_released_while_request_in_flight(addr).await?;

    // The foreground future still awaits its tracker, so the request keeps its
    // connection until the handler is released.
    ensure(
        !task.is_finished(),
        "run_with_shutdown returned before the in-flight request finished",
    )?;

    finish_request(client, &release).await?;
    timeout(FRAME_TIMEOUT, task)
        .await
        .map_err(|_| TestError::Msg("run_with_shutdown did not return after the drain".into()))?
        .map_err(|error| TestError::Msg(format!("server task failed: {error}")))??;
    Ok(())
}
