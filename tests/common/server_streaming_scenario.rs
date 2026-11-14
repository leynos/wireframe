//! Scenario construction shared by the server half-close streaming tests.
//!
//! The behavioural tests in `server_streaming_half_close.rs` all drive one
//! prepared application over the same in-memory duplex transport, with the same
//! three routes and the same paced producer. Building that scenario and reading
//! frames back from it lives here, so each test states only the behaviour it
//! checks and neither module outgrows the file-length limit.

use std::{
    io,
    sync::{Arc, Mutex, PoisonError, atomic::AtomicUsize},
    time::Duration,
};

use bytes::Bytes;
use futures::StreamExt;
use tokio::{io::DuplexStream, task::JoinHandle, time::timeout};
use wireframe::{
    app::{Envelope, ResponseHandler, WireframeApp},
    response::Response,
    serializer::{BincodeSerializer, Serializer},
};
use wireframe_testing::{TestError, TestResult};

use crate::streaming_io::{ProducerHandles, stream_frames};

/// Route whose producer paces each frame below the idle grace.
pub(crate) const PACED_ROUTE: u32 = 50;
/// Route whose producer stalls beyond the idle grace.
pub(crate) const STALLED_ROUTE: u32 = 51;
/// Route that answers immediately, so a delivered reply is observable.
pub(crate) const FAST_ROUTE: u32 = 52;
/// Correlation identifier stamped on every request in this module.
pub(crate) const REQUEST_CORRELATION: u64 = 21;
/// Duplex capacity deliberately smaller than one response frame.
const DUPLEX_CAPACITY: usize = 8;
/// Maximum frame length used by the in-memory transport.
pub(crate) const MAX_FRAME_LENGTH: usize = 2048;
/// Frames the paced producer sends, each after a sub-grace pause.
pub(crate) const PACED_FRAME_COUNT: u8 = 3;
/// Pause between paced sends: below the 30-second idle grace, but long enough
/// that the whole response outlasts one grace interval.
const PACED_SEND_PAUSE: Duration = Duration::from_secs(20);
/// Pause that exceeds the idle grace, so the response is cancelled.
const STALLED_SEND_PAUSE: Duration = Duration::from_secs(60);
/// Upper bound for a bounded state change on a loaded runner.
pub(crate) const STATE_TIMEOUT: Duration = Duration::from_secs(5);
/// Upper bound for a connection that must make progress or release.
pub(crate) const CONNECTION_SETTLE: Duration = Duration::from_secs(120);

/// Require `condition`, reporting `message` as the test failure otherwise.
pub(crate) fn ensure(condition: bool, message: impl Into<String>) -> TestResult {
    if condition {
        Ok(())
    } else {
        Err(TestError::Msg(message.into()))
    }
}

/// Retain a spawned producer so the test can await it before asserting.
fn retain_producer(producers: &ProducerHandles, producer: JoinHandle<()>) {
    producers
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .push(producer);
}

/// Handle a request by streaming frames through a response channel.
fn producer_handler(
    producers: ProducerHandles,
    frame_count: u8,
    pause: Duration,
) -> ResponseHandler<Envelope> {
    Arc::new(move |_request: &Envelope| {
        let results = Arc::new(Mutex::new(Vec::new()));
        let handles = Arc::clone(&producers);
        Box::pin(async move {
            let (sender, response) = Response::with_channel(1);
            let producer = tokio::spawn(stream_frames(sender, results, frame_count, Some(pause)));
            retain_producer(&handles, producer);
            response
        })
    })
}

/// Handle a request with an immediately available single-frame response.
fn fast_handler() -> ResponseHandler<Envelope> {
    Arc::new(|_request: &Envelope| {
        Box::pin(async { Response::Single(Envelope::new(FAST_ROUTE, None, vec![b'F'])) })
    })
}

/// A prepared connection under test.
pub(crate) struct Scenario {
    /// Client end of the duplex transport.
    pub(crate) client: DuplexStream,
    /// Spawned server task driving the prepared connection.
    pub(crate) connection: JoinHandle<io::Result<()>>,
    /// Producer tasks the handlers spawn.
    pub(crate) producers: ProducerHandles,
    /// Number of teardown hook invocations.
    pub(crate) teardowns: Arc<AtomicUsize>,
}

/// Build the three-route app and spawn its prepared connection.
pub(crate) async fn spawn_scenario() -> TestResult<Scenario> {
    let producers: ProducerHandles = Arc::new(Mutex::new(Vec::new()));
    let teardowns = Arc::new(AtomicUsize::new(0));
    let teardown_counter = Arc::clone(&teardowns);
    let app = WireframeApp::<BincodeSerializer, (), Envelope>::default()
        .route_response(
            PACED_ROUTE,
            producer_handler(Arc::clone(&producers), PACED_FRAME_COUNT, PACED_SEND_PAUSE),
        )?
        .route_response(
            STALLED_ROUTE,
            producer_handler(Arc::clone(&producers), 1, STALLED_SEND_PAUSE),
        )?
        .route_response(FAST_ROUTE, fast_handler())?
        // Teardown only runs for connections whose setup produced state, so
        // register a setup hook even though these scenarios need no state.
        .on_connection_setup(|| async {})?
        .on_connection_teardown(move |()| {
            let counter = Arc::clone(&teardown_counter);
            async move {
                counter.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            }
        })?;
    let prepared = app
        .prepare()
        .await
        .map_err(|error| TestError::Msg(format!("prepare app: {error}")))?;

    let (client, server) = tokio::io::duplex(DUPLEX_CAPACITY);
    let connection = tokio::spawn(async move { prepared.handle_connection_result(server).await });
    Ok(Scenario {
        client,
        connection,
        producers,
        teardowns,
    })
}

/// Decode one outbound frame payload into its envelope.
fn decode_envelope(bytes: &[u8]) -> TestResult<Envelope> {
    let (envelope, consumed) = BincodeSerializer
        .deserialize::<Envelope>(bytes)
        .map_err(|error| TestError::Msg(format!("deserialize frame: {error}")))?;
    ensure(
        consumed == bytes.len(),
        format!(
            "trailing bytes after envelope: consumed {consumed} of {}",
            bytes.len()
        ),
    )?;
    Ok(envelope)
}

/// Read `count` response frames and return their payloads in order.
pub(crate) async fn read_payloads<S>(frames: &mut S, count: u8) -> TestResult<Vec<Vec<u8>>>
where
    S: futures::Stream<Item = Result<Bytes, std::io::Error>> + Unpin,
{
    let mut payloads = Vec::new();
    for _ in 0..count {
        let frame = frames
            .next()
            .await
            .transpose()
            .map_err(|error| TestError::Msg(format!("read response frame: {error}")))?
            .ok_or("the response ended early")?;
        payloads.push(decode_envelope(&frame)?.payload_bytes().to_vec());
    }
    Ok(payloads)
}

/// Await the connection task within [`CONNECTION_SETTLE`], rejecting failures.
///
/// Successful frame delivery does not cover cleanup: a connection that serves
/// every frame and then stalls would hang the test. The join is therefore
/// bounded, and an expired join aborts the task and awaits its termination so
/// no detached task survives the failure.
pub(crate) async fn finish(mut connection: JoinHandle<io::Result<()>>) -> TestResult {
    let Ok(joined) = timeout(CONNECTION_SETTLE, &mut connection).await else {
        connection.abort();
        // Await the aborted task so it is provably gone before the test fails;
        // its cancellation result is the expected one and carries no diagnosis.
        drop(connection.await);
        return Err(TestError::Msg(format!(
            "connection cleanup did not finish within {CONNECTION_SETTLE:?}"
        )));
    };
    let outcome =
        joined.map_err(|error| TestError::Msg(format!("connection task failed: {error}")))?;
    ensure(
        outcome.is_ok(),
        format!("the connection must not fail: {outcome:?}"),
    )
}
