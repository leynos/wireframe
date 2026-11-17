//! Accept-loop coverage for handler panics.
//!
//! The spawner-level tests prove that a panicking connection task is logged.
//! These tests drive the real accept loop instead, so the supervision boundary
//! is exercised in the wiring the server actually uses: one panic is reported
//! once, finalizes its connection, and leaves the loop able to serve the next
//! connection.

use std::{
    net::SocketAddr,
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    },
    time::Duration,
};

use tokio::{
    io::AsyncWriteExt,
    net::{TcpListener, TcpStream},
    time::{sleep, timeout},
};
use tokio_util::{codec::LengthDelimitedCodec, sync::CancellationToken, task::TaskTracker};
use tracing_test::traced_test;
use wireframe_testing::encode_frame;

use super::{AcceptLoopOptions, BackoffConfig, PreambleHooks, SupervisorLifecycle, accept_loop};
use crate::{
    app::{Envelope, Handler, PreparedApp, WireframeApp},
    codec::LengthDelimitedFrameCodec,
    message::Message,
    serializer::BincodeSerializer,
    test_helpers::counting_teardown,
};

/// Prepared application type driven through the accept loop.
type LoopPrepared = PreparedApp<BincodeSerializer, (), Envelope, LengthDelimitedFrameCodec>;

/// Handler that panics on its first call and succeeds afterwards.
///
/// Serving a second connection after the panic proves that the loop, not only
/// the connection task, survived the failure.
fn panicking_once(calls: Arc<AtomicUsize>) -> Handler<Envelope> {
    Arc::new(move |_: &Envelope| {
        let call_count = Arc::clone(&calls);
        Box::pin(async move {
            assert!(
                call_count.fetch_add(1, Ordering::SeqCst) != 0,
                "handler boom"
            );
        })
    })
}

/// Build an application whose handler panics once, with a counted teardown.
fn panic_app(
    handler_calls: Arc<AtomicUsize>,
    teardowns: Arc<AtomicUsize>,
) -> WireframeApp<BincodeSerializer, (), Envelope> {
    WireframeApp::<BincodeSerializer, (), Envelope>::default()
        .route(1, panicking_once(handler_calls))
        .expect("register route")
        .on_connection_setup(|| async {})
        .expect("register setup hook")
        .on_connection_teardown(counting_teardown(teardowns))
        .expect("register teardown hook")
}

/// Prepare the panicking application for the accept loop.
async fn prepared_panic_app(
    handler_calls: Arc<AtomicUsize>,
    teardowns: Arc<AtomicUsize>,
) -> Arc<LoopPrepared> {
    Arc::new(
        panic_app(handler_calls, teardowns)
            .prepare()
            .await
            .expect("prepare panic app"),
    )
}

/// Encode one valid length-delimited request frame for `route`.
fn request_frame(route: u32) -> Vec<u8> {
    let envelope = Envelope::new(route, Some(7), vec![b'X']);
    let encoded = envelope.to_bytes().expect("encode envelope");
    let mut codec = LengthDelimitedCodec::builder().new_codec();
    encode_frame(&mut codec, encoded).expect("encode frame")
}

/// Connect, send one request, and close.
///
/// A panic may close the connection before the write lands, so a closed peer
/// is evidence the connection was accepted and handled rather than a failure.
async fn send_one_request(address: SocketAddr, frame: &[u8]) {
    let mut client = TcpStream::connect(address).await.expect("connect client");
    match client.write_all(frame).await {
        Ok(()) => {}
        Err(error) if is_peer_closed_error(&error) => {}
        Err(error) => panic!("client write failed: {error}"),
    }
    drop(client);
}

/// Report whether an I/O error means the peer had already closed.
fn is_peer_closed_error(error: &std::io::Error) -> bool {
    matches!(
        error.kind(),
        std::io::ErrorKind::BrokenPipe | std::io::ErrorKind::ConnectionReset
    )
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

/// A handler panic is logged once by the real accept loop, and the loop
/// continues to serve connections afterwards.
#[traced_test]
#[tokio::test]
async fn accept_loop_logs_one_panic_and_keeps_serving() {
    let handler_calls = Arc::new(AtomicUsize::new(0));
    let teardowns = Arc::new(AtomicUsize::new(0));
    let app = prepared_panic_app(Arc::clone(&handler_calls), Arc::clone(&teardowns)).await;
    let listener = Arc::new(
        TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind listener"),
    );
    let address = listener.local_addr().expect("listener address");
    let shutdown = CancellationToken::new();
    let tracker = TaskTracker::new();

    tracker.spawn(accept_loop(
        listener,
        AcceptLoopOptions::<()> {
            app,
            preamble: PreambleHooks::default(),
            shutdown: shutdown.clone(),
            tracker: tracker.clone(),
            backoff: BackoffConfig::default(),
            lifecycle: SupervisorLifecycle::new(),
        },
    ));

    let frame = request_frame(1);
    send_one_request(address, &frame).await;
    wait_until(|| teardowns.load(Ordering::SeqCst) == 1).await;
    send_one_request(address, &frame).await;
    wait_until(|| teardowns.load(Ordering::SeqCst) == 2).await;

    shutdown.cancel();
    tracker.close();
    timeout(Duration::from_secs(5), tracker.wait())
        .await
        .expect("accept loop should exit after shutdown");

    assert_eq!(
        handler_calls.load(Ordering::SeqCst),
        2,
        "the loop should serve the connection after the panic"
    );
    assert_eq!(
        teardowns.load(Ordering::SeqCst),
        2,
        "teardown should run exactly once per accepted connection"
    );
    logs_assert(|lines: &[&str]| {
        let matching = lines
            .iter()
            .filter(|line| {
                line.contains("connection task panicked") && line.contains("handler boom")
            })
            .count();
        match matching {
            1 => Ok(()),
            other => Err(format!(
                "expected exactly one supervised handler panic log, found {other}"
            )),
        }
    });
}
