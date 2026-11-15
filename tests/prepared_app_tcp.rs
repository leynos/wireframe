//! End-to-end TCP coverage for prepared applications and server integration.

#![forbid(unsafe_code)]

mod common;

use std::{
    net::SocketAddr,
    sync::{
        Arc,
        Mutex,
        atomic::{AtomicU64, AtomicUsize, Ordering},
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
    net::{TcpListener, TcpStream},
    time::timeout,
};
use wireframe::{
    app::{Envelope, Handler, WireframeApp},
    serializer::BincodeSerializer,
    server::WireframeServer,
};
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
        .ok_or_else(|| "server did not report a bound address".to_owned())?;
    let (ready_tx, ready_rx) = tokio::sync::oneshot::channel();
    let (shutdown_tx, shutdown_rx) = tokio::sync::oneshot::channel();
    let server_task = tokio::spawn(async move {
        server
            .ready_signal(ready_tx)
            .run_with_shutdown(async {
                drop(shutdown_rx.await);
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

/// Handler that panics on its first invocation and succeeds after.
fn panicking_then_healthy_handler(calls: Arc<AtomicUsize>) -> Handler<Envelope> {
    Arc::new(move |_: &Envelope| {
        let counter = Arc::clone(&calls);
        Box::pin(async move {
            assert!(counter.fetch_add(1, Ordering::SeqCst) != 0, "handler boom");
        })
    })
}

/// Wait until `teardowns` reaches `expected`.
async fn wait_for_teardown(teardowns: &AtomicUsize, expected: usize) -> TestResult<()> {
    timeout(Duration::from_secs(10), async {
        while teardowns.load(Ordering::SeqCst) < expected {
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .map_err(|_| format!("teardown count did not reach {expected}"))?;
    Ok(())
}

/// Build the app used by the panic-and-recover scenario.
///
/// Each connection receives a fresh setup ID, and teardown records the exact
/// value it was handed, so a lost or duplicated receipt is observable.
fn receipt_tracking_app(
    handler_calls: Arc<AtomicUsize>,
    next_setup_id: Arc<AtomicU64>,
    receipts: Arc<Mutex<Vec<u64>>>,
    teardowns: Arc<AtomicUsize>,
) -> TestResult<WireframeApp<BincodeSerializer, u64, Envelope>> {
    Ok(WireframeApp::<BincodeSerializer, u64, Envelope>::new()?
        .route(1, panicking_then_healthy_handler(handler_calls))?
        .on_connection_setup(move || {
            let id = next_setup_id.fetch_add(1, Ordering::SeqCst);
            async move { id }
        })?
        .on_connection_teardown(move |id: u64| {
            let logged_receipts = Arc::clone(&receipts);
            let counter = Arc::clone(&teardowns);
            async move {
                logged_receipts.lock().expect("receipt log").push(id);
                counter.fetch_add(1, Ordering::SeqCst);
            }
        })?)
}

/// Send `frame` and confirm the server closes the connection without replying.
///
/// A connection reset or broken pipe is an acceptable close: the important
/// observation is that the peer sees the transport end rather than hanging.
async fn send_frame_expecting_closure(address: SocketAddr, frame: &[u8]) -> TestResult<()> {
    let mut client = TcpStream::connect(address).await?;
    client.write_all(frame).await?;
    let mut closed = Vec::new();
    let read = timeout(Duration::from_secs(5), client.read_to_end(&mut closed))
        .await
        .expect("the server should close the panicking connection");
    match read {
        Ok(_) => Ok(()),
        Err(error)
            if matches!(
                error.kind(),
                std::io::ErrorKind::ConnectionReset | std::io::ErrorKind::BrokenPipe
            ) =>
        {
            Ok(())
        }
        Err(error) => Err(error.into()),
    }
}

/// A handler panic over TCP finalizes once and the server serves the next peer.
#[tokio::test]
async fn handler_panic_over_tcp_finalizes_and_serves_the_next_connection() -> TestResult<()> {
    let handler_calls = Arc::new(AtomicUsize::new(0));
    let teardowns = Arc::new(AtomicUsize::new(0));
    let next_setup_id = Arc::new(AtomicU64::new(1));
    let receipts = Arc::new(Mutex::new(Vec::new()));

    let app = receipt_tracking_app(
        Arc::clone(&handler_calls),
        next_setup_id,
        Arc::clone(&receipts),
        Arc::clone(&teardowns),
    )?;
    let server = WireframeServer::from_app(app)
        .workers(1)
        .bind_existing_listener(unused_listener()?)?;
    let address = server
        .local_addr()
        .ok_or_else(|| "server did not report a bound address".to_owned())?;
    let (ready_tx, ready_rx) = tokio::sync::oneshot::channel();
    let (shutdown_tx, shutdown_rx) = tokio::sync::oneshot::channel();
    let server_task = tokio::spawn(async move {
        server
            .ready_signal(ready_tx)
            .run_with_shutdown(async {
                drop(shutdown_rx.await);
            })
            .await
    });
    wait_for_server_readiness(ready_rx).await?;

    let frame = build_frame(1, vec![b'X'])?;
    send_frame_expecting_closure(address, &frame).await?;
    wait_for_teardown(&teardowns, 1).await?;

    let healthy = exchange_frame(address, frame).await?;
    check_equal(
        &healthy.is_empty(),
        &false,
        "the healthy connection should receive a response",
    )?;
    wait_for_teardown(&teardowns, 2).await?;

    shutdown_tx
        .send(())
        .map_err(|()| "server shutdown receiver was dropped")?;
    server_task.await??;
    check_equal(
        &teardowns.load(Ordering::SeqCst),
        &2usize,
        "teardown should run once per accepted connection",
    )?;
    check_equal(
        &handler_calls.load(Ordering::SeqCst),
        &2usize,
        "both connections should reach the handler",
    )?;
    let mut received = receipts.lock().expect("receipt log").clone();
    received.sort_unstable();
    check_equal(
        &received,
        &vec![1_u64, 2],
        "each connection should receive exactly one teardown receipt",
    )?;
    wait_for_listener_release(address).await
}
