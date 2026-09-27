//! Integration coverage for one-time application preparation.

mod common;

use std::{
    net::SocketAddr,
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    },
    time::Duration,
};

use common::prepared_app::{
    TestApp,
    TransformCountingMiddleware,
    build_frame,
    handler,
    response_payload,
};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::{TcpListener, TcpStream},
    sync::{Barrier, oneshot},
    time::{sleep, timeout},
};
use wireframe::{
    app::{Envelope, Handler, PreparedApp},
    serializer::BincodeSerializer,
    server::WireframeServer,
};
use wireframe_testing::{
    TestResult,
    drive_prepared_with_frames,
    unused_listener,
    wait_for_listener_release,
    wait_for_server_readiness,
};

const ROUTES: usize = 2;
const MIDDLEWARE_LAYERS: usize = 2;
const CONNECTIONS: usize = 2;

type TestPreparedApp = PreparedApp<BincodeSerializer, (), Envelope>;

/// Counter snapshots for the application connection-startup baseline.
#[derive(Clone)]
struct ConnectionStartupInstrumentation {
    factory_calls: Arc<AtomicUsize>,
    transforms: Arc<AtomicUsize>,
}

impl ConnectionStartupInstrumentation {
    /// Creates counters for factory invocations and middleware transforms.
    fn new() -> Self {
        Self {
            factory_calls: Arc::new(AtomicUsize::new(0)),
            transforms: Arc::new(AtomicUsize::new(0)),
        }
    }

    /// Returns the current connection-startup counter values.
    fn snapshot(&self) -> ConnectionStartupCounts {
        ConnectionStartupCounts {
            factory_calls: self.factory_calls.load(Ordering::SeqCst),
            transforms: self.transforms.load(Ordering::SeqCst),
        }
    }
}

#[derive(Debug, PartialEq, Eq)]
struct ConnectionStartupCounts {
    factory_calls: usize,
    transforms: usize,
}

/// Creates the test factory used to compare legacy and prepared startup work.
fn counted_app_factory(
    instrumentation: ConnectionStartupInstrumentation,
) -> impl Fn() -> TestResult<TestApp> + Clone + Send + Sync + 'static {
    move || {
        instrumentation.factory_calls.fetch_add(1, Ordering::SeqCst);
        Ok(TestApp::new()?
            .route(1, handler())?
            .route(2, handler())?
            .wrap(TransformCountingMiddleware {
                tag: b'A',
                transforms: Arc::clone(&instrumentation.transforms),
            })?
            .wrap(TransformCountingMiddleware {
                tag: b'B',
                transforms: Arc::clone(&instrumentation.transforms),
            })?)
    }
}

/// Waits until connection-startup counters reach the expected values.
async fn wait_for_counts(
    instrumentation: &ConnectionStartupInstrumentation,
    expected: &ConnectionStartupCounts,
) -> TestResult<()> {
    timeout(Duration::from_secs(1), async {
        while instrumentation.snapshot() != *expected {
            sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .map_err(|_| {
        format!(
            "connection startup counts did not reach {expected:?}; observed {:?}",
            instrumentation.snapshot()
        )
    })?;
    Ok(())
}

/// Runs server connections and waits for their startup instrumentation.
async fn run_server_connections(
    app_factory: impl Fn() -> TestResult<TestApp> + Clone + Send + Sync + 'static,
    instrumentation: &ConnectionStartupInstrumentation,
    expected: &ConnectionStartupCounts,
) -> TestResult<()> {
    let server = WireframeServer::new(app_factory)
        .workers(1)
        .bind_existing_listener(unused_listener()?)?;
    let address = server
        .local_addr()
        .ok_or_else(|| "server did not report a bound address".to_string())?;
    let (ready_tx, ready_rx) = oneshot::channel();
    let (shutdown_tx, shutdown_rx) = oneshot::channel();
    let server_task = tokio::spawn(async move {
        server
            .ready_signal(ready_tx)
            .run_with_shutdown(async {
                let _ = shutdown_rx.await;
            })
            .await
    });

    wait_for_server_readiness(ready_rx).await?;
    let frame = build_frame(1, Vec::new())?;
    let mut connections = Vec::with_capacity(CONNECTIONS);
    for _ in 0..CONNECTIONS {
        let mut connection = TcpStream::connect(address).await?;
        connection.write_all(&frame).await?;
        connections.push(connection);
    }
    let counts_result = wait_for_counts(instrumentation, expected).await;
    drop(connections);
    let shutdown_result = shutdown_tx
        .send(())
        .map_err(|()| "server shutdown receiver was dropped");
    let server_result = server_task.await;
    let listener_result = wait_for_listener_release(address).await;

    counts_result?;
    shutdown_result?;
    server_result??;
    listener_result
}

#[tokio::test]
#[expect(
    clippy::panic_in_result_fn,
    reason = "assertions make transform counts and middleware order failures explicit"
)]
async fn connection_startup_records_counts_before_and_after_preparation() -> TestResult<()> {
    let instrumentation = ConnectionStartupInstrumentation::new();
    let app_factory = counted_app_factory(instrumentation.clone());

    assert_eq!(
        instrumentation.snapshot(),
        ConnectionStartupCounts {
            factory_calls: 0,
            transforms: 0,
        }
    );

    let server_counts = ConnectionStartupCounts {
        factory_calls: 1,
        transforms: ROUTES * MIDDLEWARE_LAYERS,
    };
    run_server_connections(app_factory.clone(), &instrumentation, &server_counts).await?;
    assert_eq!(instrumentation.snapshot(), server_counts);

    let prepared: TestPreparedApp = app_factory()?
        .prepare()
        .await
        .map_err(|error| -> Box<dyn std::error::Error + Send + Sync> { Box::new(error) })?;
    let prepared_counts = ConnectionStartupCounts {
        factory_calls: 2,
        transforms: 2 * ROUTES * MIDDLEWARE_LAYERS,
    };
    assert_eq!(instrumentation.snapshot(), prepared_counts);

    let first = drive_prepared_with_frames(&prepared, vec![build_frame(1, vec![b'X'])?]).await?;
    let second = drive_prepared_with_frames(&prepared, vec![build_frame(2, vec![b'Y'])?]).await?;

    assert_eq!(instrumentation.snapshot(), prepared_counts);
    assert_eq!(response_payload(&first)?, [b'X', b'A', b'B', b'B', b'A']);
    assert_eq!(response_payload(&second)?, [b'Y', b'A', b'B', b'B', b'A']);
    Ok(())
}

#[tokio::test]
#[expect(
    clippy::panic_in_result_fn,
    reason = "assertions make prepared-connection failure behaviour explicit"
)]
async fn prepared_app_runs_teardown_after_processing_error() -> TestResult<()> {
    let teardown_calls = Arc::new(AtomicUsize::new(0));
    let teardown_counter = Arc::clone(&teardown_calls);
    let prepared = TestApp::new()?
        .on_connection_setup(|| async {})?
        .on_connection_teardown(move |()| {
            let teardown_counter = Arc::clone(&teardown_counter);
            async move {
                teardown_counter.fetch_add(1, Ordering::SeqCst);
            }
        })?
        .prepare()
        .await
        .map_err(|error| -> Box<dyn std::error::Error + Send + Sync> { Box::new(error) })?;

    let error = drive_prepared_with_frames(&prepared, vec![vec![0, 0, 0, 2, 1]])
        .await
        .expect_err("truncated frame should fail processing");
    assert_eq!(error.kind(), std::io::ErrorKind::UnexpectedEof);
    assert_eq!(teardown_calls.load(Ordering::SeqCst), 1);

    let (mut client, server) = tokio::io::duplex(64);
    client.write_all(&[0, 0, 0, 2, 1]).await?;
    client.shutdown().await?;
    prepared.handle_connection(server).await;
    assert_eq!(teardown_calls.load(Ordering::SeqCst), 2);
    Ok(())
}

#[tokio::test]
#[expect(
    clippy::panic_in_result_fn,
    reason = "assertions make concurrent prepared-service reuse explicit"
)]
async fn prepared_app_reuses_services_across_overlapping_connections() -> TestResult<()> {
    let transforms = Arc::new(AtomicUsize::new(0));
    let barrier = Arc::new(Barrier::new(CONNECTIONS));
    let handler_barrier = Arc::clone(&barrier);
    let handler: Handler<Envelope> = Arc::new(move |_: &Envelope| {
        let barrier = Arc::clone(&handler_barrier);
        Box::pin(async move {
            barrier.wait().await;
        })
    });
    let prepared = TestApp::new()?
        .route(1, handler)?
        .wrap(TransformCountingMiddleware {
            tag: b'A',
            transforms: Arc::clone(&transforms),
        })?
        .prepare()
        .await
        .map_err(|error| -> Box<dyn std::error::Error + Send + Sync> { Box::new(error) })?;
    assert_eq!(transforms.load(Ordering::SeqCst), 1);

    let first_frame = build_frame(1, vec![b'X'])?;
    let second_frame = build_frame(1, vec![b'Y'])?;
    let (first, second) = timeout(Duration::from_secs(1), async {
        tokio::join!(
            drive_prepared_with_frames(&prepared, vec![first_frame]),
            drive_prepared_with_frames(&prepared, vec![second_frame]),
        )
    })
    .await
    .map_err(|_| "prepared connections did not overlap")?;
    assert_eq!(response_payload(&first?)?, [b'X', b'A', b'A']);
    assert_eq!(response_payload(&second?)?, [b'Y', b'A', b'A']);
    assert_eq!(transforms.load(Ordering::SeqCst), 1);
    Ok(())
}

/// Bounded waits keep the real-socket test from hanging on a stalled peer.
const TCP_OPERATION_TIMEOUT: Duration = Duration::from_secs(5);

/// Accepts one TCP connection and processes it with the prepared application.
async fn serve_one_prepared_tcp_connection(
    listener: TcpListener,
    prepared: TestPreparedApp,
) -> TestResult<()> {
    let (stream, _peer) = timeout(TCP_OPERATION_TIMEOUT, listener.accept())
        .await
        .map_err(|_| "prepared TCP accept timed out".to_string())??;
    timeout(
        TCP_OPERATION_TIMEOUT,
        prepared.handle_connection_result(stream),
    )
    .await
    .map_err(|_| "prepared TCP connection processing timed out".to_string())??;
    Ok(())
}

/// Sends one encoded frame to the server and returns the decoded response.
async fn exchange_one_tcp_frame(address: SocketAddr, frame: Vec<u8>) -> TestResult<Vec<u8>> {
    let mut client = timeout(TCP_OPERATION_TIMEOUT, TcpStream::connect(address))
        .await
        .map_err(|_| "prepared TCP client connect timed out".to_string())??;
    client.write_all(&frame).await?;
    // Signal end-of-input so the server can finish the request-response cycle.
    client.shutdown().await?;
    let mut response = Vec::new();
    timeout(TCP_OPERATION_TIMEOUT, client.read_to_end(&mut response))
        .await
        .map_err(|_| "prepared TCP response read timed out".to_string())??;
    response_payload(&response)
}

#[tokio::test]
#[expect(
    clippy::panic_in_result_fn,
    reason = "assertions make real-socket prepared dispatch and transform reuse explicit"
)]
async fn prepared_app_serves_real_tcp_connection_without_retransforming() -> TestResult<()> {
    let instrumentation = ConnectionStartupInstrumentation::new();
    let prepared: TestPreparedApp = counted_app_factory(instrumentation.clone())()?
        .prepare()
        .await
        .map_err(|error| -> Box<dyn std::error::Error + Send + Sync> { Box::new(error) })?;
    let prepared_counts = ConnectionStartupCounts {
        factory_calls: 1,
        transforms: ROUTES * MIDDLEWARE_LAYERS,
    };
    assert_eq!(instrumentation.snapshot(), prepared_counts);

    let listener = TcpListener::bind("127.0.0.1:0").await?;
    let address = listener.local_addr()?;
    let server = tokio::spawn(serve_one_prepared_tcp_connection(listener, prepared));

    let payload = exchange_one_tcp_frame(address, build_frame(1, vec![b'X'])?).await?;
    // The request gains tags A then B in wrap order; the response unwinds them.
    assert_eq!(payload, [b'X', b'A', b'B', b'B', b'A']);

    let joined = timeout(TCP_OPERATION_TIMEOUT, server)
        .await
        .map_err(|_| "prepared TCP accept task did not finish".to_string())?;
    // Surfaces both a failed join and a failed connection-processing result.
    joined??;
    // Preparation transformed each route once; the connection added none.
    assert_eq!(instrumentation.snapshot(), prepared_counts);
    Ok(())
}
