//! Integration coverage for one-time application preparation.

use std::{
    convert::Infallible,
    net::SocketAddr,
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    },
    time::Duration,
};

#[path = "../common/fallible_assertions/check_equal.rs"]
mod fallible_check_equal;

use async_trait::async_trait;
use fallible_check_equal::check_equal;
use proptest::{
    prelude::*,
    test_runner::{TestCaseError, TestCaseResult},
};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::{TcpListener, TcpStream},
    runtime::Builder,
    sync::{Barrier, oneshot},
    time::{sleep, timeout},
};
use wireframe::{
    app::{Envelope, Handler, PreparedApp, WireframeApp},
    middleware::{HandlerService, Service, ServiceRequest, ServiceResponse, Transform},
    serializer::{BincodeSerializer, Serializer},
    server::WireframeServer,
};
use wireframe_testing::{
    TestResult,
    decode_frames,
    drive_prepared_with_frames,
    encode_frame,
    unused_listener,
    wait_for_listener_release,
    wait_for_server_readiness,
};

type TestApp = WireframeApp<BincodeSerializer, (), Envelope>;
type TestPreparedApp = PreparedApp<BincodeSerializer, (), Envelope>;

const ROUTES: usize = 2;
const MIDDLEWARE_LAYERS: usize = 2;
const CONNECTIONS: usize = 2;

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

struct TransformCountingMiddleware {
    tag: u8,
    transforms: Arc<AtomicUsize>,
}

struct TagService<S> {
    inner: S,
    tag: u8,
}

#[async_trait]
impl<S> Service for TagService<S>
where
    S: Service<Error = Infallible> + Send + Sync + 'static,
{
    type Error = Infallible;

    /// Adds this service's tag around the delegated request and response.
    async fn call(&self, mut request: ServiceRequest) -> Result<ServiceResponse, Self::Error> {
        request.frame_mut().push(self.tag);
        let mut response = self.inner.call(request).await?;
        response.frame_mut().push(self.tag);
        Ok(response)
    }
}

#[async_trait]
impl Transform<HandlerService<Envelope>> for TransformCountingMiddleware {
    type Output = HandlerService<Envelope>;

    /// Counts this transformation and wraps the route service with its tag.
    async fn transform(&self, service: HandlerService<Envelope>) -> Self::Output {
        self.transforms.fetch_add(1, Ordering::SeqCst);
        let id = service.id();
        HandlerService::from_service(
            id,
            TagService {
                inner: service,
                tag: self.tag,
            },
        )
    }
}

/// Builds a handler that accepts an envelope without changing it.
fn handler() -> Handler<Envelope> { Arc::new(|_envelope: &Envelope| Box::pin(async {})) }

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

/// Runs legacy server connections and waits for their startup instrumentation.
async fn run_legacy_server_connections(
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

/// Encodes an envelope into a frame for an in-process connection.
fn build_frame(id: u32, payload: Vec<u8>) -> TestResult<Vec<u8>> {
    let serializer = BincodeSerializer;
    let envelope = Envelope::new(id, Some(7), payload);
    let payload = serializer.serialize(&envelope)?;
    let mut codec = TestApp::default().length_codec();
    Ok(encode_frame(&mut codec, payload)?)
}

/// Decodes one response frame and returns its envelope payload.
fn response_payload(bytes: &[u8]) -> TestResult<Vec<u8>> {
    let frames = decode_frames(bytes)?;
    let [frame] = frames.as_slice() else {
        return Err("expected one response frame".into());
    };
    let serializer = BincodeSerializer;
    let (response, _) = serializer.deserialize::<Envelope>(frame)?;
    Ok(wireframe::app::Packet::into_parts(response).into_payload())
}

#[tokio::test]
async fn connection_startup_records_counts_before_and_after_preparation() -> TestResult<()> {
    let instrumentation = ConnectionStartupInstrumentation::new();
    let app_factory = counted_app_factory(instrumentation.clone());

    check_equal(
        &instrumentation.snapshot(),
        &ConnectionStartupCounts {
            factory_calls: 0,
            transforms: 0,
        },
        "connection counters should start at zero",
    )?;

    let legacy_counts = ConnectionStartupCounts {
        factory_calls: CONNECTIONS,
        transforms: CONNECTIONS * ROUTES * MIDDLEWARE_LAYERS,
    };
    run_legacy_server_connections(app_factory.clone(), &instrumentation, &legacy_counts).await?;
    check_equal(
        &instrumentation.snapshot(),
        &legacy_counts,
        "legacy connections should each transform the routes",
    )?;

    let prepared: TestPreparedApp = app_factory()?
        .prepare()
        .await
        .map_err(|error| -> Box<dyn std::error::Error + Send + Sync> { Box::new(error) })?;
    let prepared_counts = ConnectionStartupCounts {
        factory_calls: CONNECTIONS + 1,
        transforms: (CONNECTIONS + 1) * ROUTES * MIDDLEWARE_LAYERS,
    };
    check_equal(
        &instrumentation.snapshot(),
        &prepared_counts,
        "preparation should transform each route once",
    )?;

    let first = drive_prepared_with_frames(&prepared, vec![build_frame(1, vec![b'X'])?]).await?;
    let second = drive_prepared_with_frames(&prepared, vec![build_frame(2, vec![b'Y'])?]).await?;

    check_equal(
        &instrumentation.snapshot(),
        &prepared_counts,
        "serving prepared frames should not add transforms",
    )?;
    check_equal(
        &response_payload(&first)?,
        b"XABBA",
        "the first response should follow middleware order",
    )?;
    check_equal(
        &response_payload(&second)?,
        b"YABBA",
        "the second response should follow middleware order",
    )?;
    Ok(())
}

#[tokio::test]
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

    let error = match drive_prepared_with_frames(&prepared, vec![vec![0, 0, 0, 2, 1]]).await {
        Err(error) => error,
        Ok(response) => {
            return Err(format!(
                "truncated frame should fail processing, got response {response:?}"
            )
            .into());
        }
    };
    check_equal(
        &error.kind(),
        &std::io::ErrorKind::UnexpectedEof,
        "truncated frame should return unexpected EOF",
    )?;
    check_equal(
        &teardown_calls.load(Ordering::SeqCst),
        &1,
        "teardown should run after the in-process processing error",
    )?;

    let (mut client, server) = tokio::io::duplex(64);
    client.write_all(&[0, 0, 0, 2, 1]).await?;
    client.shutdown().await?;
    prepared.handle_connection(server).await;
    check_equal(
        &teardown_calls.load(Ordering::SeqCst),
        &2,
        "teardown should run after the duplex processing error",
    )?;
    Ok(())
}

#[tokio::test]
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
    check_equal(
        &transforms.load(Ordering::SeqCst),
        &1,
        "preparation should transform the shared route once",
    )?;

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
    check_equal(
        &response_payload(&first?)?,
        b"XAA",
        "the first overlapping response should carry both middleware tags",
    )?;
    check_equal(
        &response_payload(&second?)?,
        b"YAA",
        "the second overlapping response should carry both middleware tags",
    )?;
    check_equal(
        &transforms.load(Ordering::SeqCst),
        &1,
        "overlapping connections should not rebuild the shared route",
    )?;
    Ok(())
}

mod tcp_and_property;
