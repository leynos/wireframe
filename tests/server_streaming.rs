//! End-to-end server streaming through the integrated connection actor.
//!
//! Each scenario starts a real [`WireframeServer`] and connects a
//! [`WireframeClient`] through the test harness, so handler responses travel
//! the same path as production traffic: handler → deferred body → connection
//! actor → frame pipeline → codec → socket.
//!
//! [`WireframeServer`]: wireframe::server::WireframeServer
//! [`WireframeClient`]: wireframe::client::WireframeClient
#![cfg(not(loom))]

#[path = "fixtures/client_streaming/types.rs"]
mod stream_types;

#[path = "common/server_streaming_assertions.rs"]
mod streaming_assertions;

use std::sync::{
    Arc,
    Mutex,
    atomic::{AtomicUsize, Ordering},
};

use bytes::Bytes;
use futures::StreamExt;
use serial_test::serial;
use stream_types::{CorrelationId, MessageId, Payload, StreamTestEnvelope};
use streaming_assertions::{
    assert_active_connections,
    assert_recorded_stream,
    assert_streamed_frames,
    assert_teardown_once,
    assert_unit_route_reply,
};
use tokio::sync::mpsc;
use wireframe::{
    app::{Envelope, Handler, Packet, ResponseHandler, WireframeApp},
    codec::{FrameCodec, LengthDelimitedFrameCodec},
    hooks::{ConnectionContext, WireframeProtocol},
    response::{FrameStream, Response},
    serializer::{BincodeSerializer, Serializer},
};
use wireframe_testing::{
    TestError,
    TestResult,
    client_pair::{WireframePair, spawn_wireframe_pair},
};

/// Route identifier for a handler that streams frames from a lazy stream.
const STREAM_ROUTE: u32 = 10;
/// Route identifier for a handler that spawns a channel producer.
const SPAWNED_ROUTE: u32 = 11;
/// Route identifier for a unit handler that echoes the request payload.
const UNIT_ROUTE: u32 = 12;
/// Maximum frame length shared by the paired client and server.
const MAX_FRAME_LENGTH: usize = 2048;

/// Shared observations recorded across one paired server connection.
#[derive(Default)]
struct Observations {
    /// Payload bytes seen by the protocol `before_send` hook, in send order.
    sent_payloads: Mutex<Vec<Vec<u8>>>,
    /// Number of times the application teardown hook ran.
    teardowns: AtomicUsize,
}

impl Observations {
    /// Record one frame observed by the protocol hook.
    fn record_send(&self, frame: &Bytes) {
        self.sent_payloads
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .push(frame.to_vec());
    }

    /// Return the recorded payload bytes in send order.
    fn sent_payloads(&self) -> Vec<Vec<u8>> {
        self.sent_payloads
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .clone()
    }

    /// Count the teardown hooks that have run.
    fn teardown_count(&self) -> usize { self.teardowns.load(Ordering::SeqCst) }
}

/// Protocol that emits an id-zero terminator and records outbound frames.
struct TerminatingProtocol {
    /// Shared observations updated by the hooks.
    observations: Arc<Observations>,
}

impl TerminatingProtocol {
    /// Build the protocol around a shared observation log.
    ///
    /// The terminator is an id-zero frame carrying a fixed correlation value:
    /// [`ResponseStream`] recognises a terminator before it validates
    /// correlation, so end-of-stream frames need no per-request stamp. A
    /// multi-packet response still overwrites that value with the request
    /// correlation before the frame reaches the wire.
    ///
    /// [`ResponseStream`]: wireframe::client::ResponseStream
    const fn new(observations: Arc<Observations>) -> Self { Self { observations } }
}

impl WireframeProtocol for TerminatingProtocol {
    type Frame = Bytes;
    type ProtocolError = ();

    fn before_send(&self, frame: &mut Self::Frame, _ctx: &mut ConnectionContext) {
        self.observations.record_send(frame);
    }

    fn stream_end_frame(&self, _ctx: &mut ConnectionContext) -> Option<Self::Frame> {
        let terminator = StreamTestEnvelope::terminator(CorrelationId::new(0));
        // An unencodable terminator is treated as "no terminator"; the test
        // fixture envelope always encodes.
        let payload = terminator.serialize_to_bytes().ok()?;
        Some(LengthDelimitedFrameCodec::new(MAX_FRAME_LENGTH).wrap_payload(payload))
    }
}

/// Build the response handler that lazily yields three data frames.
fn streaming_handler() -> ResponseHandler<StreamTestEnvelope> {
    Arc::new(|_request: &StreamTestEnvelope| {
        Box::pin(async {
            let stream: FrameStream<Envelope, ()> = Box::pin(async_stream::try_stream! {
                for payload in [1_u8, 2, 3] {
                    yield Envelope::new(2, None, vec![payload]);
                }
            });
            Response::Stream(stream)
        })
    })
}

/// Send `payloads` through a response channel, stopping once the actor drops
/// the receiver.
async fn send_payloads(sender: mpsc::Sender<Envelope>, payloads: impl IntoIterator<Item = u8>) {
    for payload in payloads {
        if sender
            .send(Envelope::new(5, None, vec![payload]))
            .await
            .is_err()
        {
            return;
        }
    }
}

/// Build the response handler that streams frames from a spawned producer.
fn spawned_handler() -> ResponseHandler<StreamTestEnvelope> {
    Arc::new(|_request: &StreamTestEnvelope| {
        Box::pin(async {
            let (sender, response) = Response::with_channel(4);
            tokio::spawn(send_payloads(sender, [7_u8, 8]));
            response
        })
    })
}

/// Build the unit handler that leaves the request payload unchanged.
fn echo_handler() -> Handler<StreamTestEnvelope> {
    Arc::new(|_request: &StreamTestEnvelope| Box::pin(async {}))
}

/// Build the application shared by the streaming scenarios.
fn build_app(
    observations: &Arc<Observations>,
) -> wireframe::Result<WireframeApp<BincodeSerializer, (), StreamTestEnvelope>> {
    let teardown_observations = Arc::clone(observations);
    WireframeApp::<BincodeSerializer, (), StreamTestEnvelope>::default()
        .route_response(STREAM_ROUTE, streaming_handler())?
        .route_response(SPAWNED_ROUTE, spawned_handler())?
        .route(UNIT_ROUTE, echo_handler())?
        .with_protocol(TerminatingProtocol::new(Arc::clone(observations)))
        .on_connection_setup(|| async {})?
        .on_connection_teardown(move |()| {
            let teardown_state = Arc::clone(&teardown_observations);
            async move {
                teardown_state.teardowns.fetch_add(1, Ordering::SeqCst);
            }
        })
}

/// Start a paired server that shares `observations` across connections.
async fn start_pair(observations: &Arc<Observations>) -> TestResult<WireframePair> {
    let factory_observations = Arc::clone(observations);
    spawn_wireframe_pair(
        move || build_app(&factory_observations),
        |builder| builder.max_frame_length(MAX_FRAME_LENGTH),
    )
    .await
}

/// Send `request` and collect every data frame before the terminator.
async fn collect_stream(
    pair: &mut WireframePair,
    request: StreamTestEnvelope,
) -> TestResult<Vec<StreamTestEnvelope>> {
    let client = pair.client_mut()?;
    let mut stream = client.call_streaming::<StreamTestEnvelope>(request).await?;
    let mut frames = Vec::new();
    while let Some(frame) = stream.next().await {
        frames.push(frame?);
    }
    Ok(frames)
}

/// Build a request for `route` carrying `correlation`.
fn request_for(route: u32, correlation: Option<CorrelationId>) -> StreamTestEnvelope {
    correlation.map_or_else(
        || StreamTestEnvelope {
            id: MessageId::new(route),
            correlation_id: None,
            payload: Payload::new(Vec::new()),
        },
        |request_correlation| {
            StreamTestEnvelope::data(
                MessageId::new(route),
                request_correlation,
                Payload::new(Vec::new()),
            )
        },
    )
}

/// Serialize the id-zero terminator this protocol emits.
///
/// Multi-packet responses stamp the terminator with the request correlation,
/// so the expected bytes carry the same identifier.
fn terminator_bytes(correlation: CorrelationId) -> TestResult<Vec<u8>> {
    StreamTestEnvelope::terminator(correlation)
        .serialize_to_bytes()
        .map(|bytes| bytes.to_vec())
        .map_err(|error| TestError::Msg(format!("serialize terminator: {error}")))
}

/// Decode a recorded outbound frame into the envelope that produced it.
fn decode_recorded(frame: &[u8]) -> TestResult<Envelope> {
    BincodeSerializer
        .deserialize::<Envelope>(frame)
        .map(|(envelope, _)| envelope)
        .map_err(|error| TestError::Msg(format!("decode recorded frame: {error}")))
}

/// A streamed route emits ordered, correlated frames and one terminator.
#[tokio::test]
#[serial]
async fn streaming_route_sends_ordered_correlated_frames() -> TestResult {
    let observations = Arc::new(Observations::default());
    let mut pair = start_pair(&observations).await?;
    let correlation = CorrelationId::new(41);

    let frames = collect_stream(&mut pair, request_for(STREAM_ROUTE, Some(correlation))).await?;

    let expected: [&[u8]; 3] = [&[1], &[2], &[3]];
    assert_streamed_frames(&frames, &expected, correlation, "streamed");
    assert_active_connections(1);

    let recorded: Vec<Envelope> = observations
        .sent_payloads()
        .iter()
        .map(|frame| decode_recorded(frame))
        .collect::<TestResult<Vec<Envelope>>>()?;
    let expected_terminator = decode_recorded(&terminator_bytes(CorrelationId::new(0))?)?;
    assert_recorded_stream(&recorded, 3, &expected_terminator, correlation);

    pair.shutdown().await?;
    assert_teardown_once(&observations);
    assert_active_connections(0);
    Ok(())
}

/// A spawned producer streams channel frames through the same actor path.
#[tokio::test]
#[serial]
async fn spawned_producer_route_streams_correlated_frames() -> TestResult {
    let observations = Arc::new(Observations::default());
    let mut pair = start_pair(&observations).await?;
    let correlation = CorrelationId::new(57);

    let frames = collect_stream(&mut pair, request_for(SPAWNED_ROUTE, Some(correlation))).await?;

    let expected: [&[u8]; 2] = [&[7], &[8]];
    assert_streamed_frames(&frames, &expected, correlation, "producer");

    let recorded: Vec<Envelope> = observations
        .sent_payloads()
        .iter()
        .map(|frame| decode_recorded(frame))
        .collect::<TestResult<Vec<Envelope>>>()?;
    let terminator = decode_recorded(&terminator_bytes(correlation)?)?;
    assert_recorded_stream(&recorded, 2, &terminator, correlation);

    pair.shutdown().await?;
    assert_teardown_once(&observations);
    Ok(())
}

/// A unit route replies once, without a stream terminator.
#[tokio::test]
#[serial]
async fn unit_route_sends_exactly_one_reply() -> TestResult {
    let observations = Arc::new(Observations::default());
    let mut pair = start_pair(&observations).await?;
    let correlation = CorrelationId::new(63);
    let request = StreamTestEnvelope {
        id: MessageId::new(UNIT_ROUTE),
        correlation_id: Some(correlation),
        payload: Payload::new(vec![0xab]),
    };

    let reply: StreamTestEnvelope = pair.client_mut()?.call(&request).await?;

    let recorded = observations.sent_payloads();
    let recorded_route = recorded
        .first()
        .map(|frame| decode_recorded(frame))
        .transpose()?
        .map(|envelope| envelope.id());
    assert_unit_route_reply(&reply, correlation, recorded.len(), recorded_route);

    pair.shutdown().await?;
    assert_teardown_once(&observations);
    Ok(())
}
