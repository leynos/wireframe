//! Harness cases with a request parked inside its handler.
//!
//! These are the cases that need PRE-1's listener release: while a response is
//! in flight, the socket must already be closed once the accept loops exit, so
//! a drop or a bounded shutdown is observable without waiting for the handler.
#![cfg(not(loom))]
#![forbid(unsafe_code)]

use std::{net::SocketAddr, sync::Arc, time::Duration};

use futures::StreamExt;
use rstest::rstest;
use tokio::{
    io::AsyncWriteExt,
    net::TcpStream,
    runtime::Builder,
    sync::Notify,
    time::{Instant, timeout},
};
use tokio_util::codec::Framed;
use wireframe::{
    app::{Envelope, ResponseHandler, WireframeApp},
    codec::{FrameCodec, LengthDelimitedFrameCodec},
    response::{FrameStream, Response},
    serializer::{BincodeSerializer, Serializer},
    server::WireframeServer,
};
use wireframe_testing::{
    RunningWireframeServer,
    ServerHarnessOptions,
    ShutdownOutcome,
    TestError,
    TestResult,
    encode_frame,
    spawn_wireframe_server,
    spawn_wireframe_server_with_options,
    wait_for_listener_release,
};

/// Application type used by the parked request.
type TestApp = WireframeApp<BincodeSerializer, (), Envelope>;
/// Client stream type for the length-delimited transport.
type HeldClient = Framed<TcpStream, <LengthDelimitedFrameCodec as FrameCodec>::Decoder>;

/// Route whose handler stalls after its first frame.
const HELD_ROUTE: u32 = 63;
/// First frame payload, sent before the handler parks.
const FIRST: u8 = 5;
/// Second frame payload, sent when the test releases the handler.
const SECOND: u8 = 6;
/// Maximum frame length used by the transport.
const MAX_FRAME_LENGTH: usize = 2048;
/// Upper bound for a frame or a shutdown that must complete.
const STEP_TIMEOUT: Duration = Duration::from_secs(5);
/// Window used to show that something did not wait.
const NOT_YET: Duration = Duration::from_millis(100);
/// Bound for a repeated shutdown after a timeout, as the plan specifies.
const RETRY_WITHIN: Duration = Duration::from_millis(50);

/// Fail the test when `condition` does not hold.
fn ensure(condition: bool, message: impl Into<String>) -> TestResult {
    if condition {
        Ok(())
    } else {
        Err(TestError::Msg(message.into()))
    }
}

/// Handle a request with a stream that stalls after its first frame.
fn held_handler(started: Arc<Notify>, release: Arc<Notify>) -> ResponseHandler<Envelope> {
    Arc::new(move |_request: &Envelope| {
        let handler_started = Arc::clone(&started);
        let handler_release = Arc::clone(&release);
        Box::pin(async move {
            let stream: FrameStream<Envelope, ()> = Box::pin(async_stream::try_stream! {
                yield Envelope::new(HELD_ROUTE, None, vec![FIRST]);
                handler_started.notify_one();
                handler_release.notified().await;
                yield Envelope::new(HELD_ROUTE, None, vec![SECOND]);
            });
            Response::Stream(stream)
        })
    })
}

/// Build the parked-request application.
fn held_app(started: Arc<Notify>, release: Arc<Notify>) -> TestResult<TestApp> {
    Ok(TestApp::default().route_response(HELD_ROUTE, held_handler(started, release))?)
}

/// Encode an empty request for the held route.
fn build_request() -> TestResult<Vec<u8>> {
    let encoded = BincodeSerializer.serialize(&Envelope::new(HELD_ROUTE, Some(7), Vec::new()))?;
    let mut codec = TestApp::default().length_codec();
    Ok(encode_frame(&mut codec, encoded)?)
}

/// Send the held request, waiting until its first frame has arrived.
async fn park_request(addr: SocketAddr) -> TestResult<HeldClient> {
    let mut client = Framed::new(
        TcpStream::connect(addr).await?,
        LengthDelimitedFrameCodec::new(MAX_FRAME_LENGTH).decoder(),
    );
    client.get_mut().write_all(&build_request()?).await?;
    let frame = timeout(STEP_TIMEOUT, client.next())
        .await
        .map_err(|_| TestError::Msg("the parked request never produced a frame".into()))?
        .transpose()
        .map_err(|error| TestError::Msg(format!("read first frame: {error}")))?
        .ok_or_else(|| TestError::Msg("the connection ended before its first frame".into()))?;
    ensure(!frame.is_empty(), "the first frame carried no payload")?;
    Ok(client)
}

/// Release the handler and require its second frame.
async fn release_handler(mut client: HeldClient, release: &Notify) -> TestResult {
    release.notify_one();
    let frame = timeout(STEP_TIMEOUT, client.next())
        .await
        .map_err(|_| TestError::Msg("the parked request was never completed".into()))?
        .transpose()
        .map_err(|error| TestError::Msg(format!("read second frame: {error}")))?
        .ok_or_else(|| TestError::Msg("the connection ended before its second frame".into()))?;
    ensure(!frame.is_empty(), "the second frame carried no payload")
}

#[rstest]
#[case(false)]
#[case(true)]
#[tokio::test]
async fn drop_inside_runtime_releases_listener(#[case] park_handlers: bool) -> TestResult {
    let started = Arc::new(Notify::new());
    let release = Arc::new(Notify::new());
    let app = held_app(Arc::clone(&started), Arc::clone(&release))?;
    let server = WireframeServer::from_app(app).workers(1);
    let handle = spawn_wireframe_server(server).await?;
    let addr = handle.local_addr();
    let client = if park_handlers {
        Some(park_request(addr).await?)
    } else {
        None
    };

    let dropped_at = Instant::now();
    drop(handle);
    ensure(
        dropped_at.elapsed() < NOT_YET,
        "drop must return without awaiting the drain",
    )?;
    wait_for_listener_release(addr).await?;

    if let Some(parked_client) = client {
        release_handler(parked_client, &release).await?;
    }
    Ok(())
}

#[rstest]
#[case(false)]
#[case(true)]
#[test]
fn drop_outside_runtime_releases_listener(#[case] park_handlers: bool) -> TestResult {
    let runtime = Builder::new_multi_thread()
        .worker_threads(2)
        .enable_all()
        .build()
        .map_err(|error| TestError::Msg(format!("runtime build failed: {error}")))?;
    let (handle, addr, client, release) = runtime.block_on(async {
        let started = Arc::new(Notify::new());
        let release = Arc::new(Notify::new());
        let app = held_app(Arc::clone(&started), Arc::clone(&release))?;
        let server = WireframeServer::from_app(app).workers(1);
        let handle = spawn_wireframe_server(server).await?;
        let addr = handle.local_addr();
        let client = if park_handlers {
            Some(park_request(addr).await?)
        } else {
            None
        };
        Ok::<_, TestError>((handle, addr, client, release))
    })?;

    // The dropping thread has no runtime of its own; the runtime stays alive
    // on this thread so the supervisor can observe the cancellation.
    std::thread::spawn(move || drop(handle))
        .join()
        .map_err(|_| TestError::Msg("the dropping thread panicked".into()))?;
    runtime.block_on(wait_for_listener_release(addr))?;

    if let Some(parked_client) = client {
        runtime.block_on(release_handler(parked_client, &release))?;
    }
    Ok(())
}

#[rstest]
#[case(Duration::from_millis(300), true)]
#[case(Duration::from_secs(5), false)]
#[tokio::test]
async fn shutdown_bound_is_reported(
    #[case] bound: Duration,
    #[case] expect_timeout: bool,
) -> TestResult {
    let started = Arc::new(Notify::new());
    let release = Arc::new(Notify::new());
    let app = held_app(Arc::clone(&started), Arc::clone(&release))?;
    let server = WireframeServer::from_app(app).workers(1);
    let options = ServerHarnessOptions::default()
        .with_readiness_timeout(Some(STEP_TIMEOUT))
        .with_shutdown_timeout(Some(bound));
    let mut handle = spawn_wireframe_server_with_options(server, None, options).await?;
    let addr = handle.local_addr();
    let client = park_request(addr).await?;

    if expect_timeout {
        timeout_shutdown_case(&mut handle, addr, client, &release).await
    } else {
        generous_shutdown_case(&mut handle, client, &release).await
    }
}

/// Drive the bounded case: report the timeout, then complete after releasing.
async fn timeout_shutdown_case(
    handle: &mut RunningWireframeServer,
    addr: SocketAddr,
    client: HeldClient,
    release: &Notify,
) -> TestResult {
    let first = handle.shutdown().await;
    ensure(
        matches!(first, Err(TestError::Timeout(_))),
        format!("a short bound must report a timeout, got {first:?}"),
    )?;
    ensure(
        handle.shutdown_outcome() == Some(ShutdownOutcome::TimedOut),
        "the timeout must be recorded",
    )?;

    let repeated_at = Instant::now();
    let repeated = handle.shutdown().await;
    ensure(
        matches!(repeated, Err(TestError::Timeout(_))),
        "a repeated call after a timeout must re-check without blocking",
    )?;
    ensure(
        repeated_at.elapsed() < RETRY_WITHIN,
        "the repeated call must not add a blocking wait",
    )?;

    // The listener is released before the drain finishes, so the address is
    // refused while the request is still parked.
    wait_for_listener_release(addr).await?;
    release_handler(client, release).await?;
    await_completion_after_release(handle).await
}

/// Poll the handle until the drain completes once nothing is in flight.
async fn await_completion_after_release(handle: &mut RunningWireframeServer) -> TestResult {
    let deadline = Instant::now() + STEP_TIMEOUT;
    loop {
        if handle.shutdown().await.is_ok() {
            break;
        }
        ensure(
            Instant::now() < deadline,
            "the drain did not complete after the handler was released",
        )?;
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    ensure(
        handle.shutdown_outcome() == Some(ShutdownOutcome::Completed),
        "the concluding call must record completion",
    )
}

/// Drive the generous case: release from a second task while the drain waits.
async fn generous_shutdown_case(
    handle: &mut RunningWireframeServer,
    client: HeldClient,
    release: &Arc<Notify>,
) -> TestResult {
    let gate = Arc::clone(release);
    let releaser = tokio::spawn(async move {
        tokio::time::sleep(NOT_YET).await;
        gate.notify_one();
    });
    let result = timeout(STEP_TIMEOUT, handle.shutdown())
        .await
        .map_err(|_| TestError::Msg("a generous bound must not time out".into()))?;
    result?;
    releaser
        .await
        .map_err(|error| TestError::Msg(format!("the releasing task failed: {error}")))?;
    ensure(
        handle.shutdown_outcome() == Some(ShutdownOutcome::Completed),
        "a generous bound must record completion",
    )?;
    drop(client);
    Ok(())
}
