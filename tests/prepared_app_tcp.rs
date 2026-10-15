//! End-to-end TCP coverage for prepared applications and server integration.

mod common;

use std::{
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    },
    time::Duration,
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
    net::TcpStream,
    time::timeout,
};
use wireframe::server::WireframeServer;
use wireframe_testing::{
    TestResult,
    unused_listener,
    wait_for_listener_release,
    wait_for_server_readiness,
};

/// Bound each TCP operation so stalled server behaviour fails instead of hanging.
const TCP_OPERATION_TIMEOUT: Duration = Duration::from_secs(5);

/// Exchange one complete request-response frame with the running server.
async fn exchange_frame(address: std::net::SocketAddr, frame: Vec<u8>) -> TestResult<Vec<u8>> {
    let mut client = timeout(TCP_OPERATION_TIMEOUT, TcpStream::connect(address))
        .await
        .map_err(|_| "prepared TCP client connect timed out".to_string())??;
    timeout(TCP_OPERATION_TIMEOUT, client.write_all(&frame))
        .await
        .map_err(|_| "prepared TCP request write timed out".to_string())??;
    timeout(TCP_OPERATION_TIMEOUT, client.shutdown())
        .await
        .map_err(|_| "prepared TCP client shutdown timed out".to_string())??;
    let mut response = Vec::new();
    timeout(TCP_OPERATION_TIMEOUT, client.read_to_end(&mut response))
        .await
        .map_err(|_| "prepared TCP response read timed out".to_string())??;
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
