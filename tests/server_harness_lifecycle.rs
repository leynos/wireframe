//! Integration tests for the protocol-agnostic server lifecycle harness.
//!
//! These cases prove the public contract of
//! `wireframe_testing::server_harness`: a configured unbound server becomes a
//! ready handle with a known address, shutdown is idempotent and repeatable, a
//! failed startup reports its typed error and releases the listener, and
//! parallel handles are independent.
#![cfg(not(loom))]
#![forbid(unsafe_code)]

use std::{
    io,
    net::SocketAddr,
    sync::{Arc, atomic::AtomicUsize},
    time::Duration,
};

use async_trait::async_trait;
use futures::poll;
use rstest::rstest;
use tokio::{net::TcpStream, time::timeout};
use wireframe::{
    app::{Envelope, Handler, WireframeApp},
    middleware::{HandlerService, Transform},
    server::{ServerError, WireframeServer},
};
use wireframe_testing::{
    ShutdownOutcome,
    TestError,
    TestResult,
    echo_app_factory,
    spawn_wireframe_server,
    spawn_wireframe_server_on,
    unused_listener,
    wait_for_listener_release,
};

/// Upper bound for a shutdown that must complete promptly.
const PROMPT: Duration = Duration::from_secs(1);

/// Fail the test when `condition` does not hold.
fn ensure(condition: bool, message: impl Into<String>) -> TestResult {
    if condition {
        Ok(())
    } else {
        Err(TestError::Msg(message.into()))
    }
}

/// Assert that a connect is refused, proving the socket is gone.
async fn assert_connection_refused(addr: SocketAddr) -> TestResult {
    match TcpStream::connect(addr).await {
        Err(error) if error.kind() == io::ErrorKind::ConnectionRefused => Ok(()),
        Ok(stream) => {
            drop(stream);
            Err(format!("listener at {addr} accepted a connection after shutdown").into())
        }
        Err(error) => Err(error.into()),
    }
}

#[rstest]
#[tokio::test]
async fn spawn_then_connect_round_trip() -> TestResult {
    let counter = Arc::new(AtomicUsize::new(0));
    let server = WireframeServer::new(echo_app_factory(&counter)).workers(1);
    let mut handle = spawn_wireframe_server(server).await?;

    let addr = handle.local_addr();
    ensure(
        addr.ip().is_loopback(),
        "the harness must bind a loopback address",
    )?;
    ensure(addr.port() != 0, "the harness must report a bound port")?;
    TcpStream::connect(addr).await?;
    handle.shutdown().await
}

#[rstest]
#[tokio::test]
async fn caller_listener_keeps_its_address() -> TestResult {
    let counter = Arc::new(AtomicUsize::new(0));
    let listener = unused_listener()?;
    let reserved = listener.local_addr()?;
    let server = WireframeServer::new(echo_app_factory(&counter)).workers(1);

    let mut handle = spawn_wireframe_server_on(server, listener).await?;
    ensure(
        handle.local_addr() == reserved,
        format!(
            "the harness must keep the caller's address {reserved}, got {}",
            handle.local_addr()
        ),
    )?;
    handle.shutdown().await
}

#[rstest]
#[tokio::test]
async fn factory_error_fails_spawn_and_releases_listener() -> TestResult {
    let listener = unused_listener()?;
    let addr = listener.local_addr()?;
    let server = WireframeServer::new(|| -> Result<WireframeApp, io::Error> {
        Err(io::Error::other("refused by the test factory"))
    })
    .workers(1);

    let error = spawn_wireframe_server_on(server, listener)
        .await
        .err()
        .ok_or("a failing application factory must fail the spawn")?;
    ensure(
        matches!(error, TestError::Server(ServerError::FactoryBuild(_))),
        format!("expected a typed factory build error, got {error:?}"),
    )?;
    wait_for_listener_release(addr).await
}

#[rstest]
#[tokio::test]
async fn shutdown_is_idempotent() -> TestResult {
    let counter = Arc::new(AtomicUsize::new(0));
    let server = WireframeServer::new(echo_app_factory(&counter)).workers(1);
    let mut handle = spawn_wireframe_server(server).await?;
    let addr = handle.local_addr();

    handle.shutdown().await?;
    ensure(
        handle.shutdown_outcome() == Some(ShutdownOutcome::Completed),
        "the first shutdown must record completion",
    )?;
    assert_connection_refused(addr).await?;

    handle.shutdown().await?;
    ensure(
        handle.shutdown_outcome() == Some(ShutdownOutcome::Completed),
        "a repeated shutdown must keep the completed outcome",
    )?;
    assert_connection_refused(addr).await
}

#[rstest]
#[tokio::test]
async fn cancelled_shutdown_then_shutdown_completes() -> TestResult {
    let counter = Arc::new(AtomicUsize::new(0));
    let server = WireframeServer::new(echo_app_factory(&counter)).workers(1);
    let mut handle = spawn_wireframe_server(server).await?;
    let addr = handle.local_addr();

    {
        let pending = handle.shutdown();
        let mut pinned = std::pin::pin!(pending);
        let first_poll = poll!(pinned.as_mut());
        ensure(
            first_poll.is_pending(),
            "the first poll must leave the shutdown pending",
        )?;
    }

    timeout(PROMPT, handle.shutdown())
        .await
        .map_err(|_| TestError::Msg("the shutdown after cancellation never completed".into()))??;
    ensure(
        handle.shutdown_outcome() == Some(ShutdownOutcome::Completed),
        "the completing call must record completion",
    )?;
    assert_connection_refused(addr).await
}

#[rstest]
#[tokio::test]
async fn parallel_handles_use_distinct_listeners() -> TestResult {
    let counter_first = Arc::new(AtomicUsize::new(0));
    let mut first =
        spawn_wireframe_server(WireframeServer::new(echo_app_factory(&counter_first)).workers(1))
            .await?;
    let counter_second = Arc::new(AtomicUsize::new(0));
    let mut second =
        spawn_wireframe_server(WireframeServer::new(echo_app_factory(&counter_second)).workers(1))
            .await?;
    let counter_third = Arc::new(AtomicUsize::new(0));
    let mut third =
        spawn_wireframe_server(WireframeServer::new(echo_app_factory(&counter_third)).workers(1))
            .await?;

    let addresses = [first.local_addr(), second.local_addr(), third.local_addr()];
    let mut unique = addresses.to_vec();
    unique.sort();
    unique.dedup();
    ensure(
        unique.len() == addresses.len(),
        format!("the handles must bind distinct listeners, got {addresses:?}"),
    )?;

    first.shutdown().await?;
    TcpStream::connect(second.local_addr()).await?;
    TcpStream::connect(third.local_addr()).await?;
    second.shutdown().await?;
    third.shutdown().await
}

/// Middleware that holds application preparation until released.
struct PreparationBarrier {
    /// Fires when preparation reaches this transform.
    entered: Arc<tokio::sync::Notify>,
    /// The test and the transform meet here.
    barrier: Arc<tokio::sync::Barrier>,
}

#[async_trait]
impl Transform<HandlerService<Envelope>> for PreparationBarrier {
    type Output = HandlerService<Envelope>;

    async fn transform(&self, service: HandlerService<Envelope>) -> Self::Output {
        self.entered.notify_one();
        self.barrier.wait().await;
        service
    }
}

/// Cancelling a startup that is still preparing releases the listener.
///
/// Preparation blocks in a middleware transform, so the startup future is
/// provably in flight when it is dropped. The listener must then be released
/// without the barrier ever being crossed: the startup future owns the bound
/// server, and dropping it drops the server and its listener.
#[rstest]
#[tokio::test]
async fn cancelled_preparing_startup_releases_listener() -> TestResult {
    let entered = Arc::new(tokio::sync::Notify::new());
    let barrier = Arc::new(tokio::sync::Barrier::new(2));
    let handler: Handler<Envelope> = Arc::new(|_: &Envelope| Box::pin(async {}));
    let app = wireframe_testing::TestApp::new()?
        .route(1, Arc::clone(&handler))?
        .wrap(PreparationBarrier {
            entered: Arc::clone(&entered),
            barrier: Arc::clone(&barrier),
        })?;
    let listener = unused_listener()?;
    let addr = listener.local_addr()?;
    let server = WireframeServer::from_app(app).workers(1);

    {
        let startup = spawn_wireframe_server_on(server, listener);
        let mut pinned = std::pin::pin!(startup);
        let entered_wait = entered.notified();
        let first_poll = poll!(pinned.as_mut());
        ensure(
            first_poll.is_pending(),
            "startup must still be preparing at the first poll",
        )?;
        timeout(PROMPT, entered_wait)
            .await
            .map_err(|_| TestError::Msg("preparation never reached the barrier".into()))?;
        // Dropping the future drops the bound server; the barrier is never
        // crossed.
    }

    wait_for_listener_release(addr).await
}
