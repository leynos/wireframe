//! Shared harness for the streaming request-body integration tests.
//!
//! The harness owns three things so the scenario modules stay readable: a
//! server bound to an unused loopback port and running until a test stops it,
//! a length-delimited client that speaks the runtime's wire format, and the
//! handlers and assertions the scenarios share.

use std::{
    future::Future,
    io,
    net::SocketAddr,
    sync::{Arc, Mutex},
    time::Duration,
};

use bytes::Bytes;
use futures::{SinkExt, StreamExt};
use tokio::{
    net::TcpStream,
    sync::{Notify, oneshot},
    task::JoinHandle,
    time::timeout,
};
use tokio_util::codec::{Framed, LengthDelimitedCodec};
use wireframe::{
    app::{Envelope, MemoryBudgets, StreamingHandler, WireframeApp},
    request::{RequestBodyStream, RequestParts},
    serializer::{BincodeSerializer, Serializer},
    server::{ServerError, WireframeServer},
    test_helpers,
};
use wireframe_testing::{
    TestResult,
    unused_listener,
    wait_for_listener_release,
    wait_for_server_readiness,
};

/// Route identifier served by the streaming handler in these tests.
pub(crate) const STREAM_ROUTE: u32 = 90;
/// Route identifier served by the buffered middleware-bypass control.
pub(crate) const BUFFER_ROUTE: u32 = 1;
/// Per-connection frame budget.
pub(crate) const BUFFER_CAPACITY: usize = 512;
/// Longest any single network step may wait before the test fails.
///
/// Neither `cargo test` nor the CI job applies a global deadline, so an
/// unresponsive server would stall the run instead of reporting a regression.
pub(crate) const NETWORK_WAIT: Duration = Duration::from_secs(2);

/// Concrete application type served by the harness.
pub(crate) type TestApp = WireframeApp<BincodeSerializer, (), Envelope>;

/// Client side of one connection, framed with the runtime's length prefix.
pub(crate) type Client = Framed<TcpStream, LengthDelimitedCodec>;

/// A bound server plus the handles needed to stop it and check it exited.
pub(crate) struct RunningServer {
    /// Address the server is listening on.
    pub(crate) address: SocketAddr,
    /// Trigger that asks the server to stop accepting and drain.
    shutdown: oneshot::Sender<()>,
    /// Supervisor task, so a test can prove the server shut down cleanly.
    task: JoinHandle<Result<(), ServerError>>,
}

/// Serve `factory` on an unused loopback port until the test stops it.
pub(crate) async fn serve_with<F>(factory: F) -> TestResult<RunningServer>
where
    F: Fn() -> TestResult<TestApp> + Clone + Send + Sync + 'static,
{
    let server = WireframeServer::new(factory)
        .workers(1)
        .bind_existing_listener(unused_listener()?)?;
    let address = server
        .local_addr()
        .ok_or_else(|| "server did not report a bound address".to_owned())?;
    let (ready_tx, ready_rx) = oneshot::channel();
    let (shutdown, shutdown_rx) = oneshot::channel();
    let task = tokio::spawn(async move {
        server
            .ready_signal(ready_tx)
            .run_with_shutdown(async {
                drop(shutdown_rx.await);
            })
            .await
    });
    wait_for_server_readiness(ready_rx).await?;
    Ok(RunningServer {
        address,
        shutdown,
        task,
    })
}

/// Inbound limits a scenario server is built with.
///
/// The frame budget is both the codec's frame ceiling and the basis for the
/// default budgets, so a scenario that needs a different balance names both
/// here instead of relying on the defaults.
pub(crate) struct AppLimits {
    /// Largest frame the codec accepts, in bytes.
    pub(crate) frame_budget: usize,
    /// Explicit connection budgets, overriding the frame-derived defaults.
    pub(crate) budgets: Option<MemoryBudgets>,
}

/// Serve an application that routes each streamed body to its handler.
///
/// The message assembler is installed so a test can split a body across frames
/// using the protocol's own frame headers.
pub(crate) async fn serve_routes(
    routes: Vec<(u32, StreamingHandler)>,
) -> TestResult<RunningServer> {
    serve_routes_within(
        AppLimits {
            frame_budget: BUFFER_CAPACITY,
            budgets: None,
        },
        routes,
    )
    .await
}

/// Serve streaming routes under explicit inbound limits.
pub(crate) async fn serve_routes_within(
    limits: AppLimits,
    routes: Vec<(u32, StreamingHandler)>,
) -> TestResult<RunningServer> {
    serve_with(move || {
        let mut app: TestApp = WireframeApp::new()?.buffer_capacity(limits.frame_budget);
        if let Some(budgets) = limits.budgets {
            app = app.memory_budgets(budgets);
        }
        app = app.with_message_assembler(test_helpers::TestAssembler);
        for (route, handler) in &routes {
            app = app.streaming_route(*route, Arc::clone(handler))?;
        }
        Ok(app)
    })
    .await
}

/// Stop the server and prove it exited without error.
pub(crate) async fn finish_server(server: RunningServer) -> TestResult {
    let RunningServer {
        address,
        shutdown,
        task,
    } = server;
    shutdown
        .send(())
        .map_err(|()| "the server shutdown receiver was dropped")?;
    // The join, then the supervisor's own result: a server that exits with an
    // error must fail the test rather than pass silently.
    let joined = bounded("the server task finishing", task).await??;
    joined?;
    wait_for_listener_release(address).await?;
    Ok(())
}

/// Wait for `step` to finish, bounding it by [`NETWORK_WAIT`].
pub(crate) async fn bounded<T>(step: &str, future: impl Future<Output = T>) -> TestResult<T> {
    bounded_for(NETWORK_WAIT, step, future).await
}

/// Wait for `step` to finish, bounding it by `wait`.
///
/// A scenario whose progress depends on moving far more bytes than one network
/// step needs a longer bound without relaxing the short one elsewhere.
pub(crate) async fn bounded_for<T>(
    wait: Duration,
    step: &str,
    future: impl Future<Output = T>,
) -> TestResult<T> {
    timeout(wait, future)
        .await
        .map_err(|_| format!("{step} did not complete within {wait:?}").into())
}

/// Connect a length-delimited client to `address`.
pub(crate) async fn connect(address: SocketAddr) -> TestResult<Client> {
    let stream = bounded("connecting to the server", TcpStream::connect(address)).await??;
    Ok(Framed::new(stream, LengthDelimitedCodec::new()))
}

/// Send one envelope frame carrying `payload` to `route`.
pub(crate) async fn send_envelope(client: &mut Client, route: u32, payload: &[u8]) -> TestResult {
    let envelope = Envelope::new(route, Some(1), payload.to_vec());
    let encoded = BincodeSerializer.serialize(&envelope)?;
    bounded("sending a request frame", client.send(Bytes::from(encoded))).await??;
    Ok(())
}

/// Close the client's sending half.
///
/// The codec encodes both `Bytes` and `BytesMut`, so the sink item type is
/// named explicitly rather than inferred.
pub(crate) async fn close_client(client: &mut Client) -> TestResult {
    bounded(
        "closing the client's sending half",
        SinkExt::<Bytes>::close(client),
    )
    .await??;
    Ok(())
}

/// Read one reply frame and return its payload.
pub(crate) async fn read_reply(client: &mut Client) -> TestResult<Vec<u8>> {
    let Some(frame) = bounded("waiting for the reply frame", client.next()).await? else {
        return Err("the reply frame is missing".into());
    };
    let (envelope, _): (Envelope, usize) = BincodeSerializer.deserialize(&frame?)?;
    Ok(envelope.payload_bytes().to_vec())
}

/// Report whether any reply frame arrives within `wait`.
pub(crate) async fn reply_within(client: &mut Client, wait: Duration) -> TestResult<bool> {
    Ok(timeout(wait, client.next())
        .await
        .is_ok_and(|frame| frame.is_some()))
}

/// Handler that collects the request body and reports its first chunk.
///
/// Reporting the first chunk lets a test prove the handler ran before the rest
/// of the body was even sent.
pub(crate) fn progressing_handler(
    received: Arc<Mutex<Vec<Vec<u8>>>>,
    first_chunk_seen: Arc<Notify>,
) -> StreamingHandler {
    Arc::new(move |_parts: RequestParts, body: RequestBodyStream| {
        Box::pin(collect_body(
            body,
            Arc::clone(&received),
            Some(Arc::clone(&first_chunk_seen)),
        ))
    })
}

/// Handler that collects the request body without reporting any progress.
pub(crate) fn recording_handler(received: Arc<Mutex<Vec<Vec<u8>>>>) -> StreamingHandler {
    Arc::new(move |_parts: RequestParts, body: RequestBodyStream| {
        Box::pin(collect_body(body, Arc::clone(&received), None))
    })
}

/// Read `body` to its end, recording each chunk in arrival order.
///
/// The optional notification fires once, when the first chunk arrives, because
/// the tests that need it only care that the handler has started consuming.
pub(crate) async fn collect_body(
    mut body: RequestBodyStream,
    received: Arc<Mutex<Vec<Vec<u8>>>>,
    mut first_chunk_seen: Option<Arc<Notify>>,
) -> Option<Bytes> {
    while let Some(chunk) = body.next().await {
        let bytes = chunk.ok()?;
        if let Ok(mut chunks) = received.lock() {
            chunks.push(bytes.to_vec());
        }
        // Taking the notifier reports only the first chunk, so a later arrival
        // cannot wake a test that has already moved on.
        if let Some(notify) = first_chunk_seen.take() {
            notify.notify_one();
        }
    }
    Some(Bytes::from_static(b"done"))
}

/// Handler that records how the request body ended and never replies.
pub(crate) fn observing_handler(observed: Arc<Mutex<Option<io::ErrorKind>>>) -> StreamingHandler {
    Arc::new(move |_parts: RequestParts, body: RequestBodyStream| {
        Box::pin(observe_body_end(body, Arc::clone(&observed)))
    })
}

/// Record the body's terminal error, if any, and never produce a reply.
async fn observe_body_end(
    mut body: RequestBodyStream,
    observed: Arc<Mutex<Option<io::ErrorKind>>>,
) -> Option<Bytes> {
    while let Some(item) = body.next().await {
        if let Err(error) = item {
            record_kind(&observed, error.kind());
            return None;
        }
    }
    None
}

/// Store the observed error kind, ignoring a poisoned lock.
fn record_kind(observed: &Mutex<Option<io::ErrorKind>>, kind: io::ErrorKind) {
    if let Ok(mut slot) = observed.lock() {
        *slot = Some(kind);
    }
}

/// Assert a reply frame carried the expected payload.
pub(crate) fn assert_payload(actual: &[u8], expected: &[u8], expectation: &str) {
    assert_eq!(actual, expected, "{expectation}");
}

/// Assert the handler received exactly `expected`, one chunk at a time.
pub(crate) fn assert_chunks(actual: &[Vec<u8>], expected: &[Vec<u8>], expectation: &str) {
    assert_eq!(actual, expected, "{expectation}");
}

/// Assert the handler received a body equal to `expected` across its chunks.
pub(crate) fn assert_body(actual: &[Vec<u8>], expected: &[u8], expectation: &str) {
    assert_eq!(actual.concat(), expected, "{expectation}");
}

/// Assert the handler observed the truncated body.
pub(crate) fn assert_truncated(kind: Option<io::ErrorKind>) {
    assert_eq!(
        kind,
        Some(io::ErrorKind::UnexpectedEof),
        "the handler should observe the truncated body"
    );
}

/// Assert the server sent no reply frame.
pub(crate) fn assert_no_reply(has_reply: bool) {
    assert!(!has_reply, "an errored body sends no streaming reply");
}

/// Assert the peer was still blocked when its observation window closed.
///
/// The assertions in this module are free functions rather than statements in
/// the scenarios because a scenario returns a `Result`: a failure there must be
/// reported as an error, and `clippy::panic_in_result_fn` (denied here) rejects
/// an `assert!` inside such a function.
pub(crate) fn assert_blocked(blocked: bool, detail: &dyn std::fmt::Debug) {
    assert!(
        blocked,
        "a stalled handler must stop the connection reading, so the peer cannot finish sending: \
         {detail:?}"
    );
}

/// Assert the parked message was not answered while another body was live.
pub(crate) fn assert_not_answered_early(has_reply: bool) {
    assert!(
        !has_reply,
        "a message parked behind a live body must not be answered before it ends"
    );
}

/// Assert two counts match, reporting `expectation` on failure.
pub(crate) fn assert_count(actual: usize, expected: usize, expectation: &str) {
    assert_eq!(actual, expected, "{expectation}");
}
