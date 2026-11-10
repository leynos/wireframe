//! Back-pressure and disconnect behaviour for actor-driven server streams.
//!
//! These scenarios drive a prepared application over an in-memory duplex
//! transport, which gives the test direct control over the reader pace and the
//! moment the peer disconnects. They complement `server_streaming.rs`, which
//! covers the same path over a real loopback socket.
#![cfg(not(loom))]

#[path = "common/server_streaming_io.rs"]
mod streaming_io;

use std::{
    num::NonZeroUsize,
    sync::{
        Arc,
        Mutex,
        atomic::{AtomicUsize, Ordering},
    },
    time::Duration,
};

use bytes::Bytes;
use futures::StreamExt;
use streaming_io::{ProducerHandles, join_producers, request_bytes, stream_frames, write_frame};
use tokio::{task::JoinHandle, time::advance};
use tokio_util::codec::Framed;
use wireframe::{
    app::{Envelope, Packet, ResponseHandler, WireframeApp},
    codec::{FrameCodec, LengthDelimitedFrameCodec},
    hooks::{ConnectionContext, WireframeProtocol},
    response::Response,
    serializer::{BincodeSerializer, Serializer},
};
use wireframe_testing::{
    SlowIoConfig,
    SlowIoPacing,
    TestError,
    TestResult,
    drive_with_slow_codec_payloads,
};

/// Route identifier for the capacity-one multi-packet route.
const SLOW_ROUTE: u32 = 30;
/// Route identifier for the disconnect scenario.
const DROP_ROUTE: u32 = 31;
/// Maximum frame length used by the in-memory transport.
const MAX_FRAME_LENGTH: usize = 2048;
/// Correlation identifier stamped on every request in this module.
const REQUEST_CORRELATION: u64 = 11;
/// Pacing applied to the test reader.
const READER_CHUNK_BYTES: usize = 4;
/// Delay inserted between paced reads.
const READER_CHUNK_DELAY: Duration = Duration::from_millis(10);
/// Duplex capacity deliberately smaller than one response frame.
const DUPLEX_CAPACITY: usize = 8;

/// Protocol that terminates every response stream with an id-zero frame.
struct TerminatingProtocol;

impl WireframeProtocol for TerminatingProtocol {
    type Frame = Bytes;
    type ProtocolError = ();

    fn stream_end_frame(&self, _ctx: &mut ConnectionContext) -> Option<Self::Frame> {
        let envelope = Envelope::new(0, None, Vec::new());
        let payload = BincodeSerializer.serialize(&envelope).ok()?;
        Some(LengthDelimitedFrameCodec::new(MAX_FRAME_LENGTH).wrap_payload(Bytes::from(payload)))
    }
}

/// Delay inserted between paced producer sends.
///
/// The producer is paced so the test can observe the reader holding it back
/// before the capacity-one channel accepts the next frame.
const PRODUCER_PACING: Duration = Duration::from_millis(25);

/// Build a handler that streams `frame_count` frames from a capacity-one
/// channel.
///
/// Each send outcome is recorded, and the producer task is retained so the
/// caller can await it before asserting on the recorded outcomes.
fn channel_handler(
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
                Some(PRODUCER_PACING),
            ));
            handles
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .push(producer);
            response
        })
    })
}

/// Build an application with one streaming route and a stream terminator.
fn build_app(
    route: u32,
    send_results: Arc<Mutex<Vec<bool>>>,
    producers: ProducerHandles,
    frame_count: u8,
) -> wireframe::Result<WireframeApp<BincodeSerializer, (), Envelope>> {
    Ok(WireframeApp::<BincodeSerializer, (), Envelope>::default()
        .route_response(route, channel_handler(send_results, producers, frame_count))?
        .with_protocol(TerminatingProtocol))
}

/// Build an application that records teardown after a streamed response.
fn build_teardown_app(
    teardowns: &Arc<AtomicUsize>,
    send_results: Arc<Mutex<Vec<bool>>>,
    producers: ProducerHandles,
) -> wireframe::Result<WireframeApp<BincodeSerializer, (), Envelope>> {
    let teardown_counter = Arc::clone(teardowns);
    build_app(DROP_ROUTE, send_results, producers, 3)?
        .on_connection_setup(|| async {})?
        .on_connection_teardown(move |()| {
            let counter = Arc::clone(&teardown_counter);
            async move {
                counter.fetch_add(1, Ordering::SeqCst);
            }
        })
}

/// Decode one outbound frame payload into its envelope.
fn decode_envelope(bytes: &[u8]) -> TestResult<Envelope> {
    let (envelope, consumed) = BincodeSerializer
        .deserialize::<Envelope>(bytes)
        .map_err(|error| TestError::Msg(format!("deserialize frame: {error}")))?;
    if consumed != bytes.len() {
        return Err(TestError::Msg(format!(
            "trailing bytes after envelope: consumed {consumed} of {}",
            bytes.len()
        )));
    }
    Ok(envelope)
}

/// Assert the paced reader is still holding the streamed response back.
fn assert_drive_is_pending(drive: &JoinHandle<std::io::Result<Vec<Vec<u8>>>>) {
    assert!(
        !drive.is_finished(),
        "a paced reader must hold the streamed response back"
    );
}

/// Assert the capacity-one channel delivered ordered frames and a terminator.
fn assert_capacity_one_stream(envelopes: &[Envelope], send_results: &[bool]) {
    assert_eq!(envelopes.len(), 3, "two data frames and one terminator");
    let data: Vec<&[u8]> = envelopes
        .iter()
        .take(2)
        .map(Envelope::payload_bytes)
        .collect();
    assert_eq!(
        data,
        vec![vec![1_u8].as_slice(), vec![2_u8].as_slice()],
        "the capacity-one channel preserves order"
    );
    assert_eq!(
        envelopes.get(2).map(Packet::id),
        Some(0),
        "the stream ends with the protocol terminator"
    );
    assert_eq!(
        send_results,
        [true, true].as_slice(),
        "every frame was accepted while the peer stayed connected"
    );
}

/// Assert a mid-stream disconnect surfaced as an error and tore down once.
fn assert_disconnect_behaviour(
    first_payload: &[u8],
    outcome: &std::io::Result<()>,
    send_results: &[bool],
    teardowns: usize,
) {
    assert_eq!(
        first_payload,
        &[1_u8],
        "the first streamed frame arrives before the peer disconnects"
    );
    assert!(
        outcome.is_err(),
        "a closed peer must surface as an I/O error: {outcome:?}"
    );
    assert_eq!(
        send_results.last(),
        Some(&false),
        "the producer's next send must fail after the peer disconnects: {send_results:?}"
    );
    assert_eq!(
        teardowns, 1,
        "teardown runs exactly once after a mid-stream disconnect"
    );
}

/// A capacity-one channel streams in order while a slow reader applies
/// back-pressure.
#[tokio::test(start_paused = true)]
async fn capacity_one_channel_streams_under_reader_pressure() -> TestResult {
    let send_results = Arc::new(Mutex::new(Vec::new()));
    let producers: ProducerHandles = Arc::new(Mutex::new(Vec::new()));
    let app = build_app(
        SLOW_ROUTE,
        Arc::clone(&send_results),
        Arc::clone(&producers),
        2,
    )?;
    let codec = LengthDelimitedFrameCodec::new(MAX_FRAME_LENGTH);
    let config = SlowIoConfig::new()
        .with_capacity(DUPLEX_CAPACITY)
        .with_reader_pacing(SlowIoPacing::new(
            NonZeroUsize::new(READER_CHUNK_BYTES).expect("non-zero chunk size"),
            READER_CHUNK_DELAY,
        ))
        .validate()?;
    let request = request_bytes(SLOW_ROUTE, REQUEST_CORRELATION)?;

    let drive = tokio::spawn(async move {
        drive_with_slow_codec_payloads(app, &codec, vec![request], config).await
    });
    tokio::task::yield_now().await;
    assert_drive_is_pending(&drive);

    // Advancing the paused clock releases every reader pause in turn.
    advance(Duration::from_secs(1)).await;
    let payloads = drive.await.expect("drive task joined")?;
    let envelopes = payloads
        .iter()
        .map(|payload| decode_envelope(payload))
        .collect::<TestResult<Vec<Envelope>>>()?;

    join_producers(&producers).await?;
    let send_outcomes = send_results.lock().expect("read send outcomes").clone();
    assert_capacity_one_stream(&envelopes, &send_outcomes);
    Ok(())
}

/// Disconnecting the peer surfaces an I/O error, fails the next producer send,
/// and runs teardown exactly once.
#[tokio::test]
async fn disconnecting_peer_stops_the_stream_and_tears_down_once() -> TestResult {
    let teardowns = Arc::new(AtomicUsize::new(0));
    let send_results = Arc::new(Mutex::new(Vec::new()));
    let producers: ProducerHandles = Arc::new(Mutex::new(Vec::new()));
    let app = build_teardown_app(
        &teardowns,
        Arc::clone(&send_results),
        Arc::clone(&producers),
    )?;
    let prepared = app
        .prepare()
        .await
        .map_err(|error| TestError::Msg(format!("prepare app: {error}")))?;

    let (mut client, server) = tokio::io::duplex(DUPLEX_CAPACITY);
    let connection = tokio::spawn(async move { prepared.handle_connection_result(server).await });

    write_frame(
        &mut client,
        &request_bytes(DROP_ROUTE, REQUEST_CORRELATION)?,
    )
    .await?;

    let codec = LengthDelimitedFrameCodec::new(MAX_FRAME_LENGTH);
    let mut framed = Framed::new(client, codec.decoder());
    let first = framed
        .next()
        .await
        .transpose()
        .map_err(|error| TestError::Msg(format!("read first frame: {error}")))?
        .ok_or("stream ended before the first frame")?;
    let first_payload = decode_envelope(&first)?.payload_bytes().to_vec();

    // Dropping the client half closes the duplex, so the actor's next write
    // fails and the connection handler reports an I/O error.
    drop(framed);
    let outcome = connection
        .await
        .map_err(|error| TestError::Msg(format!("connection task failed: {error}")))?;

    join_producers(&producers).await?;
    let results = send_results.lock().expect("read send outcomes").clone();
    assert_disconnect_behaviour(
        &first_payload,
        &outcome,
        &results,
        teardowns.load(Ordering::SeqCst),
    );
    Ok(())
}
