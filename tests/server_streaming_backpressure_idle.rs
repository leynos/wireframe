//! Idle-producer disconnects, deterministic back-pressure, and post-buffer
//! observation.
//!
//! These scenarios complement `server_streaming_backpressure.rs`. A streamed
//! response whose producer never yields again must not pin the connection open
//! after the peer disappears, a producer must remain blocked while the
//! transport is applying back-pressure, and a buffered request must not stop a
//! later response from being observed.
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

use futures::StreamExt;
use streaming_io::{ProducerHandles, join_producers, request_bytes, stream_frames, write_frame};
use tokio::{
    io::DuplexStream,
    sync::mpsc,
    task::JoinHandle,
    time::{sleep, timeout},
};
use tokio_util::codec::Framed;
use wireframe::{
    app::{Envelope, ResponseHandler, WireframeApp},
    codec::{FrameCodec, LengthDelimitedFrameCodec},
    response::Response,
    serializer::{BincodeSerializer, Serializer},
};
use wireframe_testing::{TestError, TestResult};

/// Route identifier for the idle-stream scenario.
const IDLE_ROUTE: u32 = 40;
/// Route identifier for the deterministic back-pressure scenario.
const BACKPRESSURE_ROUTE: u32 = 41;
/// Route identifier for the paced post-buffer scenario.
const PACED_ROUTE: u32 = 42;
/// Correlation identifier stamped on every request in this module.
const REQUEST_CORRELATION: u64 = 12;
/// Duplex capacity deliberately smaller than one response frame.
const DUPLEX_CAPACITY: usize = 8;
/// Maximum frame length used by the in-memory transport.
const MAX_FRAME_LENGTH: usize = 2048;
/// Number of frames the producer attempts to send.
const FRAME_COUNT: u8 = 3;
/// Number of frames the paced producer sends after a buffered request.
const PACED_FRAME_COUNT: u8 = 2;
/// Time allowed for the transport to apply back-pressure.
const BACKPRESSURE_SETTLE: Duration = Duration::from_millis(150);
/// Upper bound for a bounded state change on a loaded runner.
const STATE_TIMEOUT: Duration = Duration::from_secs(5);
/// Pause between paced producer sends, longer than the idle grace.
///
/// A response that is still observed tolerates this pause. A response that
/// inherited a stale unobservable signal is cancelled at the grace instead, so
/// the test distinguishes the two.
const PACED_SEND_PAUSE: Duration = Duration::from_secs(60);
/// Upper bound for a connection that must release after a peer leaves.
///
/// Comfortably longer than the half-close grace so the assertion fails on a
/// pinned connection rather than a slow one.
const CONNECTION_SETTLE: Duration = Duration::from_secs(120);

/// Take-once slot keeping an idle stream's sender alive.
type IdleSender = Arc<Mutex<Option<mpsc::Sender<Envelope>>>>;

/// Wait until `condition` holds, failing after `timeout`.
///
/// A loaded runner may not have reached the expected state by the time a fixed
/// sleep ends. Polling turns that race into a bounded wait while still failing
/// when the state never arrives.
async fn wait_until(limit: Duration, condition: impl Fn() -> bool, message: &str) -> TestResult {
    let deadline = tokio::time::Instant::now() + limit;
    while !condition() {
        if tokio::time::Instant::now() >= deadline {
            return Err(TestError::Msg(message.to_owned()));
        }
        tokio::time::sleep(Duration::from_millis(5)).await;
    }
    Ok(())
}

/// Require `condition`, reporting `message` as the test failure otherwise.
fn ensure(condition: bool, message: impl Into<String>) -> TestResult {
    if condition {
        Ok(())
    } else {
        Err(TestError::Msg(message.into()))
    }
}

/// Handle a request by installing a channel that never produces a frame.
fn idle_handler(installed: IdleSender) -> ResponseHandler<Envelope> {
    Arc::new(move |_request: &Envelope| {
        let sender_slot = Arc::clone(&installed);
        Box::pin(async move {
            let (sender, response) = Response::with_channel(1);
            let mut slot = sender_slot.lock().unwrap_or_else(PoisonError::into_inner);
            *slot = Some(sender);
            response
        })
    })
}

/// Retain a spawned producer so the test can await it before asserting.
fn retain_producer(producers: &ProducerHandles, producer: JoinHandle<()>) {
    producers
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .push(producer);
}

/// Handle a request by streaming frames as fast as the channel accepts them.
fn producer_handler(
    send_results: Arc<Mutex<Vec<bool>>>,
    producers: ProducerHandles,
    frame_count: u8,
) -> ResponseHandler<Envelope> {
    Arc::new(move |_request: &Envelope| {
        let results = Arc::clone(&send_results);
        let handles = Arc::clone(&producers);
        Box::pin(async move {
            let (sender, response) = Response::with_channel(1);
            let producer = tokio::spawn(stream_frames(sender, results, frame_count, None));
            retain_producer(&handles, producer);
            response
        })
    })
}

/// Handle a request by streaming frames with a pause longer than the grace.
///
/// The pause only stays inside the budget while the peer is still observable,
/// which is what the post-buffer scenario checks.
fn paced_handler(
    send_results: Arc<Mutex<Vec<bool>>>,
    producers: ProducerHandles,
    frame_count: u8,
) -> ResponseHandler<Envelope> {
    Arc::new(move |_request: &Envelope| {
        let results = Arc::clone(&send_results);
        let handles = Arc::clone(&producers);
        Box::pin(async move {
            let (sender, response) = Response::with_channel(1);
            let producer = tokio::spawn(stream_frames(
                sender,
                results,
                frame_count,
                Some(PACED_SEND_PAUSE),
            ));
            retain_producer(&handles, producer);
            response
        })
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

/// Wait until the handler has installed its pending stream.
async fn wait_for_installed_stream(installed: &IdleSender) -> TestResult {
    wait_until(
        STATE_TIMEOUT,
        || {
            installed
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .is_some()
        },
        "handler did not install the pending stream",
    )
    .await
}

/// Complete an installed idle stream so the connection can dispatch again.
fn complete_installed_stream(installed: &IdleSender) -> TestResult {
    let sender = installed
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .take()
        .ok_or("no stream is installed")?;
    // Dropping the sender closes the channel, so the response completes.
    drop(sender);
    Ok(())
}

/// A prepared idle-stream connection under test.
struct IdleConnection {
    /// Client end of the duplex transport.
    client: DuplexStream,
    /// Spawned server task driving the prepared connection.
    connection: JoinHandle<std::io::Result<()>>,
    /// Number of teardown hook invocations.
    teardowns: Arc<AtomicUsize>,
    /// Slot holding the pending stream's sender.
    installed: IdleSender,
}

/// Build an idle-stream app, prepare it, and spawn its connection.
///
/// Both idle-stream scenarios need the same app wiring, so it lives here
/// rather than being repeated in each test.
async fn spawn_idle_connection() -> TestResult<IdleConnection> {
    let teardowns = Arc::new(AtomicUsize::new(0));
    let installed: IdleSender = Arc::new(Mutex::new(None));
    let teardown_counter = Arc::clone(&teardowns);
    let app = WireframeApp::<BincodeSerializer, (), Envelope>::default()
        .route_response(IDLE_ROUTE, idle_handler(Arc::clone(&installed)))?
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
    Ok(IdleConnection {
        client,
        connection,
        teardowns,
        installed,
    })
}

/// A peer that disconnects while its stream is still idle must release the
/// connection instead of waiting forever for a producer that never yields.
#[tokio::test(start_paused = true)]
async fn idle_stream_is_cancelled_when_the_peer_disconnects() -> TestResult {
    let IdleConnection {
        mut client,
        connection,
        teardowns,
        installed,
    } = spawn_idle_connection().await?;

    write_frame(
        &mut client,
        &request_bytes(IDLE_ROUTE, REQUEST_CORRELATION)?,
    )
    .await?;
    wait_for_installed_stream(&installed).await?;

    // Dropping the peer closes both halves: the server sees EOF while the
    // producer is still pending. The half-close grace must cancel the idle
    // stream once it expires (advanced automatically by the paused clock).
    drop(client);
    let outcome = connection
        .await
        .map_err(|error| TestError::Msg(format!("connection task failed: {error}")))?;

    ensure(
        outcome.is_ok(),
        format!("a cancelled idle stream is not an I/O failure: {outcome:?}"),
    )?;
    ensure(
        teardowns.load(Ordering::SeqCst) == 1,
        "teardown runs once after an idle-stream disconnect",
    )?;
    Ok(())
}

/// A pipelined request must not hide a later disconnect from an idle stream.
///
/// The read half stops being polled once the second request is buffered, so a
/// peer that leaves afterwards is not seen by the reader. The response wait is
/// still bounded, so the connection task, its gauge guard, and teardown cannot
/// stay pending forever.
#[tokio::test(start_paused = true)]
async fn buffered_request_does_not_pin_an_idle_stream() -> TestResult {
    let IdleConnection {
        mut client,
        connection,
        teardowns,
        installed,
    } = spawn_idle_connection().await?;

    write_frame(
        &mut client,
        &request_bytes(IDLE_ROUTE, REQUEST_CORRELATION)?,
    )
    .await?;
    wait_for_installed_stream(&installed).await?;

    // The second request is read while the first response is still idle. The
    // write completes only once the server has drained the duplex buffer, so
    // the request is buffered with the read half no longer polled.
    write_frame(
        &mut client,
        &request_bytes(IDLE_ROUTE, REQUEST_CORRELATION)?,
    )
    .await?;
    drop(client);

    let outcome = timeout(CONNECTION_SETTLE, connection)
        .await
        .map_err(|_| TestError::Msg("a buffered request pinned the idle stream".into()))?
        .map_err(|error| TestError::Msg(format!("connection task failed: {error}")))?;

    ensure(
        outcome.is_ok(),
        format!("a cancelled idle stream is not an I/O failure: {outcome:?}"),
    )?;
    ensure(
        teardowns.load(Ordering::SeqCst) == 1,
        "teardown runs once after a disconnect behind a buffered request",
    )?;
    Ok(())
}

/// Buffering a request must not make later responses unobservable.
///
/// The buffered frame cancels only the per-dispatch signal. The next response
/// is therefore watched again, so a producer that pauses longer than the idle
/// grace still delivers both frames. A connection-wide signal would already be
/// cancelled when that response starts, so the grace would expire inside the
/// pause and truncate the stream.
#[tokio::test(start_paused = true)]
async fn buffered_request_does_not_disarm_later_observation() -> TestResult {
    let send_results = Arc::new(Mutex::new(Vec::new()));
    let producers: ProducerHandles = Arc::new(Mutex::new(Vec::new()));
    let installed: IdleSender = Arc::new(Mutex::new(None));
    let app = WireframeApp::<BincodeSerializer, (), Envelope>::default()
        .route_response(IDLE_ROUTE, idle_handler(Arc::clone(&installed)))?
        .route_response(
            PACED_ROUTE,
            paced_handler(
                Arc::clone(&send_results),
                Arc::clone(&producers),
                PACED_FRAME_COUNT,
            ),
        )?;
    let prepared = app
        .prepare()
        .await
        .map_err(|error| TestError::Msg(format!("prepare app: {error}")))?;
    let (mut client, server) = tokio::io::duplex(DUPLEX_CAPACITY);
    let connection = tokio::spawn(async move { prepared.handle_connection_result(server).await });

    // Both requests are queued before the first response is installed, so the
    // second is already readable when the first dispatch stops polling the read
    // half. Writing it later would race the idle response's completion, and the
    // frame would then be read by the next dispatch instead of being buffered.
    write_frame(
        &mut client,
        &request_bytes(IDLE_ROUTE, REQUEST_CORRELATION)?,
    )
    .await?;
    write_frame(
        &mut client,
        &request_bytes(PACED_ROUTE, REQUEST_CORRELATION)?,
    )
    .await?;
    wait_for_installed_stream(&installed).await?;
    // Let the connection task re-poll its select with the queued frame before
    // the idle response is completed. Completing it first would let the
    // handling future win the biased select, and the frame would then be read
    // by the next dispatch instead of being buffered behind this one.
    sleep(STATE_TIMEOUT).await;
    complete_installed_stream(&installed)?;

    let codec = LengthDelimitedFrameCodec::new(MAX_FRAME_LENGTH);
    let mut framed = Framed::new(client, codec.decoder());
    // The second frame only arrives because the peer is observed again. Under
    // the idle grace it would never be produced: the grace expires inside the
    // producer's pause. One deadline covers the whole read because a paused
    // clock advances to the nearest timer.
    let envelopes = timeout(CONNECTION_SETTLE, async {
        let mut envelopes = Vec::new();
        for _ in 0..PACED_FRAME_COUNT {
            let frame = framed
                .next()
                .await
                .transpose()
                .map_err(|error| TestError::Msg(format!("read response frame: {error}")))?
                .ok_or("the response ended early")?;
            envelopes.push(decode_envelope(&frame)?);
        }
        Ok::<_, TestError>(envelopes)
    })
    .await
    .map_err(|_| TestError::Msg("a paced frame never arrived".into()))??;

    let data: Vec<Vec<u8>> = envelopes
        .iter()
        .map(|envelope| envelope.payload_bytes().to_vec())
        .collect();
    ensure(
        data == vec![vec![1_u8], vec![2_u8]],
        format!("both paced frames must be delivered in order: {data:?}"),
    )?;

    drop(framed);
    let outcome = connection
        .await
        .map_err(|error| TestError::Msg(format!("connection task failed: {error}")))?;
    ensure(
        outcome.is_ok(),
        format!("the connection must not fail: {outcome:?}"),
    )?;
    join_producers(&producers).await?;
    Ok(())
}

/// The producer's third send must stay blocked until the paced reader makes
/// room, proving the actor transmits transport back-pressure to its source.
#[tokio::test]
async fn producer_send_blocks_until_the_reader_progresses() -> TestResult {
    let send_results = Arc::new(Mutex::new(Vec::new()));
    let producers: ProducerHandles = Arc::new(Mutex::new(Vec::new()));
    let app = WireframeApp::<BincodeSerializer, (), Envelope>::default().route_response(
        BACKPRESSURE_ROUTE,
        producer_handler(
            Arc::clone(&send_results),
            Arc::clone(&producers),
            FRAME_COUNT,
        ),
    )?;
    let prepared = app
        .prepare()
        .await
        .map_err(|error| TestError::Msg(format!("prepare app: {error}")))?;

    let (mut client, server) = tokio::io::duplex(DUPLEX_CAPACITY);
    let connection = tokio::spawn(async move { prepared.handle_connection_result(server).await });

    write_frame(
        &mut client,
        &request_bytes(BACKPRESSURE_ROUTE, REQUEST_CORRELATION)?,
    )
    .await?;
    // Wait for the two accepted sends rather than assuming a fixed sleep is
    // long enough on a loaded runner.
    wait_until(
        STATE_TIMEOUT,
        || {
            send_results
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .len()
                >= 2
        },
        "the first two producer sends never completed",
    )
    .await?;
    // Give a third send time to complete if back-pressure were absent.
    sleep(BACKPRESSURE_SETTLE).await;

    // The first frame occupies the transport, the second occupies the
    // capacity-one channel, and the third send has nowhere to go.
    let recorded = send_results
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .clone();
    ensure(
        recorded == vec![true, true],
        format!("the third producer send must remain blocked: {recorded:?}"),
    )?;
    ensure(
        !connection.is_finished(),
        "the connection must stay pending while the reader has not progressed",
    )?;

    // Dropping the unread peer releases the blocked write and the producer.
    drop(client);
    connection
        .await
        .map_err(|error| TestError::Msg(format!("connection task failed: {error}")))
        .map(drop)?;
    join_producers(&producers).await?;
    Ok(())
}
