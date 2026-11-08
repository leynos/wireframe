//! Coverage for connection task spawning and panic recovery behaviour.

use std::{
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    },
    time::Duration,
};

#[cfg(feature = "metrics")]
use metrics_util::debugging::{DebugValue, DebuggingRecorder};
use rstest::rstest;
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::{TcpListener, TcpStream},
    runtime::Builder,
    time::{sleep, timeout},
};
use tokio_util::{codec::LengthDelimitedCodec, task::TaskTracker};
use tracing_test::traced_test;
use wireframe_testing::encode_frame;

use super::*;
use crate::{
    app::{Handler, WireframeApp},
    codec::LengthDelimitedFrameCodec,
    message::Message,
    serializer::BincodeSerializer,
    server::test_util::factory,
    test_helpers::counting_teardown,
};

/// Panic payload shapes a connection handler can raise.
///
/// Kept as an enum rather than a closure so the setup callback stays `Fn`
/// and each case selects a payload the logger must render differently.
#[derive(Clone, Copy, Debug)]
enum PanicPayload {
    /// A string message, logged verbatim.
    Message,
    /// A non-string payload, logged with its `Debug` representation.
    NonString,
}

impl PanicPayload {
    fn raise(self) -> ! {
        match self {
            Self::Message => panic!("boom"),
            Self::NonString => std::panic::panic_any(5_u32),
        }
    }
}

/// Panics in connection handlers are logged and do not tear down the worker.
///
/// `#[traced_test]` must precede `#[rstest]`: parameterized cases are emitted
/// into a child module, and the injected `logs_assert` helper is only in scope
/// for the function body it is expanded into.
#[traced_test]
#[rstest]
#[case::string_payload(PanicPayload::Message, "panic=boom")]
#[case::non_string_payload(PanicPayload::NonString, "panic=Any")]
#[tokio::test]
async fn spawn_connection_task_logs_panic(
    factory: impl Fn() -> WireframeApp + Send + Sync + Clone + 'static,
    #[case] payload: PanicPayload,
    #[case] expected_panic: &str,
) {
    let app = Arc::new(
        factory()
            .on_connection_setup(move || async move { payload.raise() })
            .expect("failed to install panic setup callback")
            .prepare()
            .await
            .expect("prepare app"),
    );
    let tracker = TaskTracker::new();
    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind listener");
    let addr = listener.local_addr().expect("listener.local_addr");

    let handle = tokio::spawn({
        let task_tracker = tracker.clone();
        let task_app = Arc::clone(&app);
        async move {
            let (stream, _) = listener.accept().await.expect("accept");
            spawn_connection_task(
                stream,
                task_app,
                PreambleHooks::<()>::default(),
                &task_tracker,
            );
            task_tracker.close();
            task_tracker.wait().await;
        }
    });

    let mut client = TcpStream::connect(addr).await.expect("connect");
    let peer_addr = client.local_addr().expect("client.local_addr");
    // The panicking task may close the connection before this write lands, so
    // a closed-peer error is itself evidence that the connection was handled.
    match client.write_all(&[0; 8]).await {
        Ok(()) => {}
        Err(error) if is_peer_closed_error(&error) => {}
        Err(error) => panic!("client write failed: {error}"),
    }
    drop(client);

    handle.await.expect("join connection task driver");
    tokio::task::yield_now().await;

    logs_assert(|lines: &[&str]| {
        lines
            .iter()
            .find(|line| {
                line.contains("connection task panicked")
                    && line.contains(expected_panic)
                    && line.contains(&format!("peer_addr=Some({peer_addr})"))
            })
            .map(|_| ())
            .ok_or_else(|| format!("panic log not found for {expected_panic}"))
    });
}

/// Report whether an I/O error means the peer had already closed.
fn is_peer_closed_error(error: &std::io::Error) -> bool {
    matches!(
        error.kind(),
        std::io::ErrorKind::BrokenPipe | std::io::ErrorKind::ConnectionReset
    )
}

/// Prepared application type used by the supervision scenario.
type SupervisedPrepared =
    crate::app::PreparedApp<BincodeSerializer, (), Envelope, LengthDelimitedFrameCodec>;

/// Build a handler that panics on its first invocation and succeeds after.
fn panicking_then_healthy_handler(call_count: Arc<AtomicUsize>) -> Handler<Envelope> {
    Arc::new(move |_: &Envelope| {
        let calls = Arc::clone(&call_count);
        Box::pin(async move {
            assert!(calls.fetch_add(1, Ordering::SeqCst) != 0, "handler boom");
        })
    })
}

/// Build a prepared application whose handler panics on its first call.
async fn prepared_supervision_app(
    handler_calls: Arc<AtomicUsize>,
    teardowns: Arc<AtomicUsize>,
) -> Result<SupervisedPrepared, Box<dyn std::error::Error + Send + Sync>> {
    WireframeApp::<BincodeSerializer, (), Envelope>::default()
        .route(1, panicking_then_healthy_handler(handler_calls))?
        .on_connection_setup(|| async {})?
        .on_connection_teardown(counting_teardown(teardowns))?
        .prepare()
        .await
        .map_err(|error| -> Box<dyn std::error::Error + Send + Sync> { Box::new(error) })
}

/// Encode one valid length-delimited request frame for `route`.
fn request_frame(route: u32) -> Vec<u8> {
    let envelope = Envelope::new(route, Some(7), vec![b'X']);
    let encoded = envelope.to_bytes().expect("encode envelope");
    let mut codec = LengthDelimitedCodec::builder().new_codec();
    encode_frame(&mut codec, encoded).expect("encode frame")
}

/// Poll `predicate` until it holds, failing the test after a short deadline.
async fn wait_until(predicate: impl Fn() -> bool) {
    timeout(Duration::from_secs(5), async {
        while !predicate() {
            sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .expect("condition was not met before the timeout");
}

/// Endpoint that serves supervised connections for one scenario.
struct SupervisionHarness {
    listener: TcpListener,
    app: Arc<SupervisedPrepared>,
    tracker: TaskTracker,
    frame: Vec<u8>,
}

impl SupervisionHarness {
    /// Connect one client, accept it, and spawn its connection task.
    async fn spawn_client(&self) -> TcpStream {
        let address = self.listener.local_addr().expect("listener address");
        let mut client = TcpStream::connect(address).await.expect("connect client");
        let (stream, _) = self.listener.accept().await.expect("accept client");
        spawn_connection_task(
            stream,
            Arc::clone(&self.app),
            PreambleHooks::<()>::default(),
            &self.tracker,
        );
        client
            .write_all(&self.frame)
            .await
            .expect("write request frame");
        client
    }

    /// Serve the connection whose handler panics and await its finalization.
    async fn serve_panicking_connection(&self, teardowns: &Arc<AtomicUsize>) {
        let client = self.spawn_client().await;
        drop(client);
        wait_until(|| teardowns.load(Ordering::SeqCst) == 1).await;
    }

    /// Serve a healthy connection and check that it receives a response.
    async fn serve_healthy_connection(&self) {
        let mut client = self.spawn_client().await;
        client.shutdown().await.expect("shutdown healthy client");
        let mut response = [0_u8; 1];
        let read = timeout(Duration::from_secs(5), client.read(&mut response))
            .await
            .expect("healthy connection should respond")
            .expect("read healthy response");
        assert!(read > 0, "the healthy connection should be served");
    }
}

/// Serve one panicking connection, then a healthy one, on the same app.
async fn drive_supervision_scenario(
    app: Arc<SupervisedPrepared>,
    handler_calls: Arc<AtomicUsize>,
    teardowns: Arc<AtomicUsize>,
) {
    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind listener");
    let harness = SupervisionHarness {
        listener,
        app,
        tracker: TaskTracker::new(),
        frame: request_frame(1),
    };

    harness.serve_panicking_connection(&teardowns).await;
    harness.serve_healthy_connection().await;

    harness.tracker.close();
    harness.tracker.wait().await;

    assert_eq!(
        handler_calls.load(Ordering::SeqCst),
        2,
        "both connections should reach the handler"
    );
    assert_eq!(
        teardowns.load(Ordering::SeqCst),
        2,
        "teardown should run exactly once per accepted connection"
    );
}

/// A handler panic after setup finalizes once, is supervised, and does not
/// poison the next connection.
///
/// The scenario uses a current-thread runtime so the spawned connection tasks
/// run on the test thread and the thread-local metric recorder and tracing
/// subscriber observe their emissions.
#[traced_test]
#[test]
fn handler_panic_finalizes_and_supervises_the_next_connection() {
    let handler_calls = Arc::new(AtomicUsize::new(0));
    let teardowns = Arc::new(AtomicUsize::new(0));
    let runtime = Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("build current-thread runtime");
    let app = Arc::new(
        runtime
            .block_on(prepared_supervision_app(
                Arc::clone(&handler_calls),
                Arc::clone(&teardowns),
            ))
            .expect("prepare supervision app"),
    );

    #[cfg(feature = "metrics")]
    let recorder = DebuggingRecorder::new();
    #[cfg(feature = "metrics")]
    let snapshotter = recorder.snapshotter();

    #[cfg(feature = "metrics")]
    ::metrics::with_local_recorder(&recorder, || {
        runtime.block_on(drive_supervision_scenario(app, handler_calls, teardowns));
    });
    #[cfg(not(feature = "metrics"))]
    runtime.block_on(drive_supervision_scenario(app, handler_calls, teardowns));

    logs_assert(|lines: &[&str]| {
        lines
            .iter()
            .find(|line| line.contains("connection task panicked") && line.contains("handler boom"))
            .map(|_| ())
            .ok_or_else(|| "the handler panic was not supervised".to_owned())
    });

    #[cfg(feature = "metrics")]
    {
        let samples = snapshotter.snapshot().into_vec();
        assert!(
            samples.iter().any(|(key, _, _, value)| {
                key.key().name() == crate::metrics::CONNECTION_PANICS
                    && matches!(value, DebugValue::Counter(count) if *count == 1)
            }),
            "assertion failed: exactly one connection panic should be counted"
        );
    }
}
