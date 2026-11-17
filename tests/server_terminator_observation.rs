//! Terminator delivery after the peer stops being observed.
//!
//! A streamed response ends with a protocol terminator, and the peer can stop
//! being observed before that last frame is written: it may half-close while
//! still reading, or pipeline another request so the read half is no longer
//! polled. Both observation paths must only bound the writes, never truncate
//! them, so the frames and the terminator still arrive. They must also bound a
//! terminator the peer never accepts, so an unreadable peer cannot pin the
//! connection. The stalled-write half of that contract is covered by the unit
//! tables in `frame_handling`.
#![cfg(not(loom))]
#![forbid(unsafe_code)]

use std::{io, sync::Arc, time::Duration};

use bytes::Bytes;
use futures::StreamExt;
use tokio::{
    io::{AsyncWriteExt, DuplexStream},
    task::JoinHandle,
    time::timeout,
};
use tokio_util::codec::Framed;
use wireframe::{
    app::{Envelope, ResponseHandler, WireframeApp},
    codec::{FrameCodec, LengthDelimitedFrameCodec},
    hooks::{ConnectionContext, WireframeProtocol},
    response::{FrameStream, Response},
    serializer::{BincodeSerializer, Serializer},
};
use wireframe_testing::{TestError, TestResult, encode_frame};

/// Application type shared by the request builder and the connection under
/// test.
type TestApp = WireframeApp<BincodeSerializer, (), Envelope>;

/// Route whose handler streams numbered frames.
const STREAM_ROUTE: u32 = 70;
/// Route whose handler answers with one immediate frame.
const FOLLOW_UP_ROUTE: u32 = 71;
/// Route whose handler streams no frames, so only a terminator is written.
const EMPTY_ROUTE: u32 = 72;
/// Frames the streaming producer sends before it ends.
const FRAME_COUNT: u8 = 2;
/// Correlation identifier stamped on every request in this module.
const REQUEST_CORRELATION: u64 = 31;
/// Maximum frame length used by the in-memory transport.
const MAX_FRAME_LENGTH: usize = 2048;
/// Duplex capacity large enough to hold the whole exchange.
const DUPLEX_CAPACITY: usize = 4096;
/// Duplex capacity small enough that no frame write can complete unread.
const BLOCKED_CAPACITY: usize = 1;
/// Upper bound for a frame that must already be in flight.
const FRAME_TIMEOUT: Duration = Duration::from_secs(20);
/// Upper bound for the connection task to finish once its frames are written.
const CONNECTION_SETTLE: Duration = Duration::from_secs(120);
/// Pause between streamed frames, so a pipelined request is read first.
const FRAME_PAUSE: Duration = Duration::from_millis(50);
/// Payload of the protocol terminator frame.
const TERMINATOR: &[u8] = b"END";
/// The production idle grace, restated because the constant is crate-private.
///
/// Keep in step with `PEER_CLOSED_RESPONSE_GRACE` in `src/connection/drive.rs`.
const IDLE_GRACE: Duration = Duration::from_secs(30);
/// Slack allowed on either side of the idle grace.
///
/// The clock is paused, so the deadline is exact and this only absorbs
/// scheduling at the same virtual instant. It deliberately stays far below the
/// grace itself, so a longer deadline cannot satisfy the upper bound.
const GRACE_TOLERANCE: Duration = Duration::from_millis(1);

/// Require `condition`, reporting `message` as the test failure otherwise.
fn ensure(condition: bool, message: impl Into<String>) -> TestResult {
    if condition {
        Ok(())
    } else {
        Err(TestError::Msg(message.into()))
    }
}

/// Protocol that terminates every streamed response.
struct TerminatingProtocol;

impl WireframeProtocol for TerminatingProtocol {
    type Frame = Bytes;
    type ProtocolError = ();

    fn stream_end_frame(&self, _ctx: &mut ConnectionContext) -> Option<Self::Frame> {
        Some(Bytes::from_static(TERMINATOR))
    }
}

/// Handle a request by streaming numbered frames with a short pause.
fn stream_handler() -> ResponseHandler<Envelope> {
    Arc::new(|_request: &Envelope| {
        Box::pin(async {
            let stream: FrameStream<Envelope, ()> = Box::pin(async_stream::try_stream! {
                for payload in 1..=FRAME_COUNT {
                    tokio::time::sleep(FRAME_PAUSE).await;
                    yield Envelope::new(STREAM_ROUTE, None, vec![payload]);
                }
            });
            Response::Stream(stream)
        })
    })
}

/// Handle a request with an immediately available single-frame response.
fn follow_up_handler() -> ResponseHandler<Envelope> {
    Arc::new(|_request: &Envelope| {
        Box::pin(async { Response::Single(Envelope::new(FOLLOW_UP_ROUTE, None, vec![b'F'])) })
    })
}

/// Handle a request by streaming no frames at all.
///
/// The response still ends with the protocol terminator, so a terminator the
/// peer never accepts is the only write left in flight.
fn empty_stream_handler() -> ResponseHandler<Envelope> {
    Arc::new(|_request: &Envelope| {
        Box::pin(async {
            let stream: FrameStream<Envelope, ()> = futures::stream::empty().boxed();
            Response::Stream(stream)
        })
    })
}

/// A prepared connection under test.
struct Scenario {
    /// Client end of the duplex transport.
    client: DuplexStream,
    /// Spawned server task driving the prepared connection.
    connection: JoinHandle<io::Result<()>>,
}

/// Build the three-route app and spawn its prepared connection.
async fn spawn_scenario() -> TestResult<Scenario> { spawn_scenario_with(DUPLEX_CAPACITY).await }

/// Build the app and spawn its prepared connection over `capacity` bytes.
///
/// A capacity smaller than one encoded frame is what makes every response
/// write block until the peer reads, which is how the abandoned-terminator
/// scenarios keep a write pending.
async fn spawn_scenario_with(capacity: usize) -> TestResult<Scenario> {
    let app = TestApp::default()
        .route_response(STREAM_ROUTE, stream_handler())?
        .route_response(FOLLOW_UP_ROUTE, follow_up_handler())?
        .route_response(EMPTY_ROUTE, empty_stream_handler())?
        .with_protocol(TerminatingProtocol);
    let prepared = app
        .prepare()
        .await
        .map_err(|error| TestError::Msg(format!("prepare app: {error}")))?;

    let (client, server) = tokio::io::duplex(capacity);
    let connection = tokio::spawn(async move { prepared.handle_connection_result(server).await });
    Ok(Scenario { client, connection })
}

/// Encode an empty request for `route` in the transport's frame format.
fn request_bytes(route: u32) -> TestResult<Vec<u8>> {
    let encoded = BincodeSerializer.serialize(&Envelope::new(
        route,
        Some(REQUEST_CORRELATION),
        Vec::new(),
    ))?;
    let mut codec = TestApp::default().length_codec();
    Ok(encode_frame(&mut codec, encoded)?)
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

/// Read the next frame, failing the test if none arrives in time.
async fn next_frame<S>(frames: &mut S) -> TestResult<Bytes>
where
    S: futures::Stream<Item = Result<Bytes, io::Error>> + Unpin,
{
    timeout(FRAME_TIMEOUT, frames.next())
        .await
        .map_err(|_| TestError::Msg("the response stalled while the peer was reading".into()))?
        .transpose()
        .map_err(|error| TestError::Msg(format!("read response frame: {error}")))?
        .ok_or_else(|| TestError::Msg("the response ended before its terminator".into()))
}

/// Read `count` response frames and return their payloads in order.
async fn read_payloads<S>(frames: &mut S, count: u8) -> TestResult<Vec<Vec<u8>>>
where
    S: futures::Stream<Item = Result<Bytes, io::Error>> + Unpin,
{
    let mut payloads = Vec::new();
    for _ in 0..count {
        payloads.push(
            decode_envelope(&next_frame(frames).await?)?
                .payload_bytes()
                .to_vec(),
        );
    }
    Ok(payloads)
}

/// Await the connection task, rejecting a transport failure or a stalled join.
///
/// Frame reads are bounded individually, but the final join is not: a
/// connection that delivers every frame and then stalls in cleanup would hang
/// the test. An expired join aborts the task and awaits its termination, so a
/// cleanup regression fails within a bounded interval and leaves nothing
/// running behind it.
async fn finish(mut connection: JoinHandle<io::Result<()>>) -> TestResult {
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

/// A half-closed peer still receives every frame and the terminator.
///
/// The peer stops sending but keeps reading, so the actor observes EOF and
/// bounds its idle waits. Every write still completes, which must include the
/// terminator after the stream ends.
#[tokio::test]
async fn a_half_closed_peer_still_receives_the_stream_terminator() -> TestResult {
    let Scenario { client, connection } = spawn_scenario().await?;

    let (reader, mut writer) = tokio::io::split(client);
    writer.write_all(&request_bytes(STREAM_ROUTE)?).await?;
    // Close only the sending half: the peer still reads every response frame.
    writer.shutdown().await?;

    let codec = LengthDelimitedFrameCodec::new(MAX_FRAME_LENGTH);
    let mut framed = Framed::new(reader, codec.decoder());
    let payloads = read_payloads(&mut framed, FRAME_COUNT).await?;
    ensure(
        payloads == vec![vec![1], vec![2]],
        format!("every frame must be delivered in order: {payloads:?}"),
    )?;
    let terminator = next_frame(&mut framed).await?;
    ensure(
        terminator.as_ref() == TERMINATOR,
        format!("a half-close must not drop the terminator: {terminator:?}"),
    )?;

    drop(framed);
    finish(connection).await
}

/// A pipelined request still receives the first response's terminator.
///
/// The second request is buffered while the first response is still in flight,
/// so the read half stops being polled. That observation change must bound the
/// first response rather than truncate it: its terminator is still written, and
/// the buffered request is then served.
#[tokio::test]
async fn a_buffered_request_still_receives_the_stream_terminator() -> TestResult {
    let Scenario { client, connection } = spawn_scenario().await?;

    let (reader, mut writer) = tokio::io::split(client);
    writer.write_all(&request_bytes(STREAM_ROUTE)?).await?;
    writer.write_all(&request_bytes(FOLLOW_UP_ROUTE)?).await?;

    let codec = LengthDelimitedFrameCodec::new(MAX_FRAME_LENGTH);
    let mut framed = Framed::new(reader, codec.decoder());
    let payloads = read_payloads(&mut framed, FRAME_COUNT).await?;
    ensure(
        payloads == vec![vec![1], vec![2]],
        format!("every frame must be delivered in order: {payloads:?}"),
    )?;
    let terminator = next_frame(&mut framed).await?;
    ensure(
        terminator.as_ref() == TERMINATOR,
        format!("a buffered request must not drop the terminator: {terminator:?}"),
    )?;
    let follow_up = decode_envelope(&next_frame(&mut framed).await?)?
        .payload_bytes()
        .to_vec();
    ensure(
        follow_up == vec![b'F'],
        format!("the buffered request must still be served: {follow_up:?}"),
    )?;

    // The connection stays open for a further request until the peer stops
    // sending, so close the sending half before waiting for it to end.
    writer.shutdown().await?;
    drop(framed);
    finish(connection).await
}

/// Await the connection task, requiring it to end at the idle grace.
///
/// The peer never reads, so the terminator write cannot complete on its own.
/// The clock is paused, so a bounded wait latches onto the grace timer: the
/// connection must end there, neither earlier nor later.
async fn finish_at_the_grace(connection: JoinHandle<io::Result<()>>) -> TestResult {
    let window_start = tokio::time::Instant::now();
    let outcome = timeout(IDLE_GRACE + GRACE_TOLERANCE, connection)
        .await
        .map_err(|_| {
            TestError::Msg("a blocked terminator pinned the connection past the idle grace".into())
        })?;

    let elapsed = window_start.elapsed();
    ensure(
        elapsed >= IDLE_GRACE,
        format!("the blocked terminator must be released at the grace, not before: {elapsed:?}"),
    )?;
    outcome
        .map_err(|error| TestError::Msg(format!("connection task failed: {error}")))?
        .map_err(|error| TestError::Msg(format!("the connection must not fail: {error}")))
}

/// A half-closed peer releases a terminator it never reads.
///
/// The peer stops sending and never reads, and the transport holds less than
/// one frame, so the terminator write cannot complete. The half-close must
/// bound that write: the connection ends at the idle grace rather than pinning
/// the connection task.
#[tokio::test(start_paused = true)]
async fn a_half_closed_peer_releases_a_blocked_terminator() -> TestResult {
    let Scenario { client, connection } = spawn_scenario_with(BLOCKED_CAPACITY).await?;

    let (reader, mut writer) = tokio::io::split(client);
    writer.write_all(&request_bytes(EMPTY_ROUTE)?).await?;
    writer.shutdown().await?;

    // Kept alive but never polled, so the terminator write stays blocked.
    let _unread = reader;
    finish_at_the_grace(connection).await
}

/// A buffered request releases a terminator the peer never reads.
///
/// The second request is buffered while the first response's terminator is
/// still blocked, so the read half stops being polled. That observation change
/// must bound the terminator write: the connection ends at the idle grace
/// instead of waiting for a peer that will never read.
#[tokio::test(start_paused = true)]
async fn a_buffered_request_releases_a_blocked_terminator() -> TestResult {
    let Scenario { client, connection } = spawn_scenario_with(BLOCKED_CAPACITY).await?;

    let (reader, mut writer) = tokio::io::split(client);
    writer.write_all(&request_bytes(EMPTY_ROUTE)?).await?;
    writer.write_all(&request_bytes(FOLLOW_UP_ROUTE)?).await?;

    // Kept alive but never polled, so the terminator write stays blocked.
    let _unread = reader;
    finish_at_the_grace(connection).await
}
