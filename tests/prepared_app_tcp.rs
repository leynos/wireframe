//! End-to-end TCP coverage for prepared applications and server integration.

mod common;

use std::sync::{
    Arc,
    atomic::{AtomicUsize, Ordering},
};

#[path = "common/fallible_assertions/check_equal.rs"]
mod fallible_check_equal;

use common::prepared_app::{
    TestApp,
    TransformCountingMiddleware,
    build_frame,
    handler,
    response_payload,
};
use fallible_check_equal::check_equal;
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::{TcpListener, TcpStream},
};
use wireframe::server::WireframeServer;
use wireframe_testing::{
    TestResult,
    unused_listener,
    wait_for_listener_release,
    wait_for_server_readiness,
};

/// Prepared applications serve TCP connections without rebuilding middleware.
#[tokio::test]
async fn prepared_app_serves_tcp_connection_without_retransforming() -> TestResult<()> {
    let transforms = Arc::new(AtomicUsize::new(0));
    let prepared = TestApp::new()?
        .route(1, handler())?
        .wrap(TransformCountingMiddleware {
            tag: b'A',
            transforms: Arc::clone(&transforms),
        })?
        .prepare()
        .await
        .map_err(|error| -> Box<dyn std::error::Error + Send + Sync> { Box::new(error) })?;
    check_equal(
        &transforms.load(Ordering::SeqCst),
        &1,
        "preparation should transform the route once",
    )?;

    let listener = TcpListener::bind("127.0.0.1:0").await?;
    let address = listener.local_addr()?;
    let server = tokio::spawn(async move {
        let (stream, _) = listener.accept().await?;
        prepared.handle_connection_result(stream).await
    });

    let mut client = TcpStream::connect(address).await?;
    client.write_all(&build_frame(1, vec![b'X'])?).await?;
    client.shutdown().await?;
    let mut response = Vec::new();
    client.read_to_end(&mut response).await?;
    server.await??;

    check_equal(
        &response_payload(&response)?,
        b"XAA",
        "the TCP response should carry both middleware tags",
    )?;
    check_equal(
        &transforms.load(Ordering::SeqCst),
        &1,
        "serving the TCP connection should not rebuild the route",
    )?;
    Ok(())
}

/// Exchange one complete request-response frame with the running server.
async fn exchange_frame(address: std::net::SocketAddr, frame: Vec<u8>) -> TestResult<Vec<u8>> {
    let mut client = TcpStream::connect(address).await?;
    client.write_all(&frame).await?;
    client.shutdown().await?;
    let mut response = Vec::new();
    client.read_to_end(&mut response).await?;
    response_payload(&response)
}

/// `from_app` prepares its supplied application once and shares it over TCP.
#[tokio::test]
async fn from_app_serves_concurrent_tcp_connections_from_one_prepared_root() -> TestResult<()> {
    let transforms = Arc::new(AtomicUsize::new(0));
    let app = TestApp::new()?
        .route(1, handler())?
        .wrap(TransformCountingMiddleware {
            tag: b'A',
            transforms: Arc::clone(&transforms),
        })?;
    let server = WireframeServer::from_app(app)
        .workers(2)
        .bind_existing_listener(unused_listener()?)?;
    let address = server
        .local_addr()
        .ok_or_else(|| "server did not report a bound address".to_string())?;
    let (ready_tx, ready_rx) = tokio::sync::oneshot::channel();
    let (shutdown_tx, shutdown_rx) = tokio::sync::oneshot::channel();
    let server_task = tokio::spawn(async move {
        server
            .ready_signal(ready_tx)
            .run_with_shutdown(async {
                let _ = shutdown_rx.await;
            })
            .await
    });

    wait_for_server_readiness(ready_rx).await?;
    let frame = build_frame(1, vec![b'X'])?;
    let (first, second) = tokio::join!(
        exchange_frame(address, frame.clone()),
        exchange_frame(address, frame)
    );
    check_equal(
        &first?,
        b"XAA",
        "the first concurrent connection should follow middleware order",
    )?;
    check_equal(
        &second?,
        b"XAA",
        "the second concurrent connection should follow middleware order",
    )?;
    check_equal(
        &transforms.load(Ordering::SeqCst),
        &1,
        "from_app should prepare the application once for both workers",
    )?;

    shutdown_tx
        .send(())
        .map_err(|()| "server shutdown receiver was dropped")?;
    server_task.await??;
    wait_for_listener_release(address).await
}
