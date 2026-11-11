//! Half-close delivery and post-cancellation shutdown.
//!
//! A peer that closes only its sending half still reads, so a response that
//! keeps producing frames must be delivered through a prepared connection. When
//! a producer instead stalls past the idle grace, the response is cancelled
//! without a terminator, so the connection must then stop rather than serve the
//! request buffered behind it.
#![cfg(not(loom))]

#[path = "common/server_streaming_io.rs"]
mod streaming_io;

use std::{
    sync::{
        Arc,
        Mutex,
        PoisonError,
        atomic::{AtomicUsize, Ordering},
    },
    time::Duration,
};

use bytes::Bytes;
use futures::StreamExt;
use streaming_io::{ProducerHandles, join_producers, request_bytes, stream_frames, write_frame};
use tokio::{
    io::{AsyncWriteExt, DuplexStream},
    task::JoinHandle,
    time::timeout,
};
use tokio_util::codec::Framed;
use wireframe::{
    app::{Envelope, ResponseHandler, WireframeApp},
    codec::{FrameCodec, LengthDelimitedFrameCodec},
    response::Response,
    serializer::{BincodeSerializer, Serializer},
};
use wireframe_testing::{TestError, TestResult};

/// Route whose producer paces each frame below the idle grace.
const PACED_ROUTE: u32 = 50;
/// Route whose producer stalls beyond the idle grace.
const STALLED_ROUTE: u32 = 51;
/// Route that answers immediately, so a delivered reply is observable.
const FAST_ROUTE: u32 = 52;
/// Correlation identifier stamped on every request in this module.
const REQUEST_CORRELATION: u64 = 21;
/// Duplex capacity deliberately smaller than one response frame.
const DUPLEX_CAPACITY: usize = 8;
/// Maximum frame length used by the in-memory transport.
const MAX_FRAME_LENGTH: usize = 2048;
/// Frames the paced producer sends, each after a sub-grace pause.
const PACED_FRAME_COUNT: u8 = 3;
/// Pause between paced sends: below the 30-second idle grace, but long enough
/// that the whole response outlasts one grace interval.
const PACED_SEND_PAUSE: Duration = Duration::from_secs(20);
/// Pause that exceeds the idle grace, so the response is cancelled.
const STALLED_SEND_PAUSE: Duration = Duration::from_secs(60);
/// Upper bound for a bounded state change on a loaded runner.
const STATE_TIMEOUT: Duration = Duration::from_secs(5);
/// Upper bound for a connection that must make progress or release.
const CONNECTION_SETTLE: Duration = Duration::from_secs(120);

/// Require `condition`, reporting `message` as the test failure otherwise.
fn ensure(condition: bool, message: impl Into<String>) -> TestResult {
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
struct Scenario {
    /// Client end of the duplex transport.
    client: DuplexStream,
    /// Spawned server task driving the prepared connection.
    connection: JoinHandle<std::io::Result<()>>,
    /// Producer tasks the handlers spawn.
    producers: ProducerHandles,
    /// Number of teardown hook invocations.
    teardowns: Arc<AtomicUsize>,
}

/// Build the three-route app and spawn its prepared connection.
async fn spawn_scenario() -> TestResult<Scenario> {
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
                counter.fetch_add(1, Ordering::SeqCst);
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
async fn read_payloads<S>(frames: &mut S, count: u8) -> TestResult<Vec<Vec<u8>>>
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

/// Await the connection task, rejecting a transport failure.
async fn finish(connection: JoinHandle<std::io::Result<()>>) -> TestResult {
    let outcome = connection
        .await
        .map_err(|error| TestError::Msg(format!("connection task failed: {error}")))?;
    ensure(
        outcome.is_ok(),
        format!("the connection must not fail: {outcome:?}"),
    )
}

/// A half-close must not truncate a response that keeps making progress.
///
/// The peer stops sending but keeps reading, and each producer pause is shorter
/// than the idle grace even though the whole response outlasts one. Delivery
/// therefore proves a valid half-close still works through a prepared
/// connection, because the deadline is renewed per frame.
#[tokio::test(start_paused = true)]
async fn half_close_still_delivers_a_progressing_stream() -> TestResult {
    let Scenario {
        client,
        connection,
        producers,
        ..
    } = spawn_scenario().await?;

    let (reader, mut writer) = tokio::io::split(client);
    write_frame(
        &mut writer,
        &request_bytes(PACED_ROUTE, REQUEST_CORRELATION)?,
    )
    .await?;
    // Close only the sending half: the peer still reads every response frame.
    writer.shutdown().await?;

    let codec = LengthDelimitedFrameCodec::new(MAX_FRAME_LENGTH);
    let mut framed = Framed::new(reader, codec.decoder());
    let payloads = timeout(
        CONNECTION_SETTLE,
        read_payloads(&mut framed, PACED_FRAME_COUNT),
    )
    .await
    .map_err(|_| TestError::Msg("a half-close truncated a progressing stream".into()))??;
    ensure(
        payloads == vec![vec![1], vec![2], vec![3]],
        format!("every paced frame must be delivered in order: {payloads:?}"),
    )?;

    drop(framed);
    finish(connection).await?;
    join_producers(&producers).await
}

/// A cancelled response must end the connection before the buffered request.
///
/// The idle grace expires while a second request waits behind the stalled
/// stream. The cancelled stream has no terminator, so serving that request
/// would make its response indistinguishable from the truncated stream: the
/// connection closes instead and the client sees end-of-file.
#[tokio::test(start_paused = true)]
async fn a_cancelled_stream_ends_the_connection_before_the_buffered_request() -> TestResult {
    let Scenario {
        client,
        connection,
        teardowns,
        producers,
    } = spawn_scenario().await?;

    let (reader, mut writer) = tokio::io::split(client);
    write_frame(
        &mut writer,
        &request_bytes(STALLED_ROUTE, REQUEST_CORRELATION)?,
    )
    .await?;
    let codec = LengthDelimitedFrameCodec::new(MAX_FRAME_LENGTH);
    let mut framed = Framed::new(reader, codec.decoder());
    // The first frame arrives immediately; the producer then stalls for longer
    // than the idle grace.
    let first = timeout(STATE_TIMEOUT, read_payloads(&mut framed, 1))
        .await
        .map_err(|_| TestError::Msg("the stalled stream produced no first frame".into()))??;
    ensure(
        first == vec![vec![1]],
        format!("the first stalled frame must be delivered: {first:?}"),
    )?;

    // The fast request is buffered behind the stalled response, so the read
    // half stops being polled and the response is bounded by the idle grace.
    write_frame(
        &mut writer,
        &request_bytes(FAST_ROUTE, REQUEST_CORRELATION)?,
    )
    .await?;

    let next = timeout(CONNECTION_SETTLE, framed.next())
        .await
        .map_err(|_| TestError::Msg("the cancelled connection stayed open".into()))?;
    ensure(
        next.is_none(),
        format!("no response may follow a cancelled stream: {next:?}"),
    )?;

    finish(connection).await?;
    ensure(
        teardowns.load(Ordering::SeqCst) == 1,
        "teardown runs once after a cancelled stream closes the connection",
    )?;
    join_producers(&producers).await
}
