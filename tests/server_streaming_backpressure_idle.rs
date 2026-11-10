//! Idle-producer disconnects and deterministic back-pressure.
//!
//! These scenarios complement `server_streaming_backpressure.rs`. A streamed
//! response whose producer never yields again must not pin the connection open
//! after the peer disappears, and a producer must remain blocked while the
//! transport is applying back-pressure.
#![cfg(not(loom))]

use std::{
    sync::{
        Arc,
        Mutex,
        PoisonError,
        atomic::{AtomicUsize, Ordering},
    },
    time::Duration,
};

use bytes::{BufMut, BytesMut};
use tokio::{
    io::{AsyncWriteExt, DuplexStream},
    sync::mpsc,
    task::JoinHandle,
    time::{sleep, timeout},
};
use wireframe::{
    app::{Envelope, ResponseHandler, WireframeApp},
    response::Response,
    serializer::{BincodeSerializer, Serializer},
};
use wireframe_testing::{TestError, TestResult};

/// Route identifier for the idle-stream scenario.
const IDLE_ROUTE: u32 = 40;
/// Route identifier for the deterministic back-pressure scenario.
const BACKPRESSURE_ROUTE: u32 = 41;
/// Correlation identifier stamped on every request in this module.
const REQUEST_CORRELATION: u64 = 12;
/// Duplex capacity deliberately smaller than one response frame.
const DUPLEX_CAPACITY: usize = 8;
/// Number of frames the producer attempts to send.
const FRAME_COUNT: u8 = 3;
/// Time allowed for the transport to apply back-pressure.
const BACKPRESSURE_SETTLE: Duration = Duration::from_millis(150);
/// Upper bound for a connection that must release after a peer leaves.
///
/// Comfortably longer than the half-close grace so the assertion fails on a
/// pinned connection rather than a slow one.
const CONNECTION_SETTLE: Duration = Duration::from_secs(120);

/// Spawned producer tasks retained so tests can await them.
type ProducerHandles = Arc<Mutex<Vec<JoinHandle<()>>>>;

/// Take-once slot keeping an idle stream's sender alive.
type IdleSender = Arc<Mutex<Option<mpsc::Sender<Envelope>>>>;

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

/// Send `frame_count` frames, recording whether each send reached the channel.
///
/// Kept out of the handler closure so the producer's loop stays shallow enough
/// for the workspace nesting lint.
async fn stream_frames(
    sender: mpsc::Sender<Envelope>,
    send_results: Arc<Mutex<Vec<bool>>>,
    frame_count: u8,
) {
    for payload in 1..=frame_count {
        let sent = sender
            .send(Envelope::new(2, None, vec![payload]))
            .await
            .is_ok();
        send_results
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(sent);
    }
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
            let producer = tokio::spawn(stream_frames(sender, results, frame_count));
            handles
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .push(producer);
            response
        })
    })
}

/// Encode a request envelope as the frame payload the client would send.
fn request_bytes(route: u32) -> TestResult<Vec<u8>> {
    let envelope = Envelope::new(route, Some(REQUEST_CORRELATION), Vec::new());
    BincodeSerializer
        .serialize(&envelope)
        .map_err(|error| TestError::Msg(format!("serialize request: {error}")))
}

/// Write one length-delimited frame to the duplex peer.
async fn write_frame(stream: &mut DuplexStream, payload: &[u8]) -> TestResult {
    let length = u32::try_from(payload.len())
        .map_err(|error| TestError::Msg(format!("frame too long: {error}")))?;
    let mut framed = BytesMut::with_capacity(4 + payload.len());
    framed.put_u32(length);
    framed.extend_from_slice(payload);
    stream.write_all(&framed).await?;
    Ok(())
}

/// Wait until the handler has installed its pending stream.
async fn wait_for_installed_stream(installed: &IdleSender) -> TestResult {
    for _ in 0..1_000 {
        if installed
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .is_some()
        {
            return Ok(());
        }
        tokio::task::yield_now().await;
    }
    Err(TestError::Msg(
        "handler did not install the pending stream".into(),
    ))
}

/// Await every producer task the handler spawned.
async fn join_producers(producers: &ProducerHandles) -> TestResult {
    let handles: Vec<JoinHandle<()>> = producers
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .drain(..)
        .collect();
    for handle in handles {
        handle
            .await
            .map_err(|error| TestError::Msg(format!("producer task failed: {error}")))?;
    }
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

    write_frame(&mut client, &request_bytes(IDLE_ROUTE)?).await?;
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

    write_frame(&mut client, &request_bytes(IDLE_ROUTE)?).await?;
    wait_for_installed_stream(&installed).await?;

    // The second request is read while the first response is still idle. The
    // write completes only once the server has drained the duplex buffer, so
    // the request is buffered with the read half no longer polled.
    write_frame(&mut client, &request_bytes(IDLE_ROUTE)?).await?;
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

    write_frame(&mut client, &request_bytes(BACKPRESSURE_ROUTE)?).await?;
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
