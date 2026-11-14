//! Terminator delivery after the peer stops being observed.
//!
//! A streamed response ends with a protocol terminator, and the peer can stop
//! being observed before that last frame is written: it may half-close while
//! still reading, or pipeline another request so the read half is no longer
//! polled. Both observation paths must only bound the writes, never truncate
//! them, so the frames and the terminator still arrive. The stalled-write half
//! of that contract is covered by the unit tables in `frame_handling`.
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
/// Frames the streaming producer sends before it ends.
const FRAME_COUNT: u8 = 2;
/// Correlation identifier stamped on every request in this module.
const REQUEST_CORRELATION: u64 = 31;
/// Maximum frame length used by the in-memory transport.
const MAX_FRAME_LENGTH: usize = 2048;
/// Duplex capacity large enough to hold the whole exchange.
const DUPLEX_CAPACITY: usize = 4096;
/// Upper bound for a frame that must already be in flight.
const FRAME_TIMEOUT: Duration = Duration::from_secs(20);
/// Pause between streamed frames, so a pipelined request is read first.
const FRAME_PAUSE: Duration = Duration::from_millis(50);
/// Payload of the protocol terminator frame.
const TERMINATOR: &[u8] = b"END";

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

/// A prepared connection under test.
struct Scenario {
    /// Client end of the duplex transport.
    client: DuplexStream,
    /// Spawned server task driving the prepared connection.
    connection: JoinHandle<io::Result<()>>,
}

/// Build the two-route app and spawn its prepared connection.
async fn spawn_scenario() -> TestResult<Scenario> {
    let app = TestApp::default()
        .route_response(STREAM_ROUTE, stream_handler())?
        .route_response(FOLLOW_UP_ROUTE, follow_up_handler())?
        .with_protocol(TerminatingProtocol);
    let prepared = app
        .prepare()
        .await
        .map_err(|error| TestError::Msg(format!("prepare app: {error}")))?;

    let (client, server) = tokio::io::duplex(DUPLEX_CAPACITY);
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

/// Await the connection task, rejecting a transport failure.
async fn finish(connection: JoinHandle<io::Result<()>>) -> TestResult {
    let outcome = connection
        .await
        .map_err(|error| TestError::Msg(format!("connection task failed: {error}")))?;
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
