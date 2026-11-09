//! `ServerStreamingWorld` fixture for server-side streaming BDD tests.
//!
//! The world starts a real paired server whose routes use `route_response`,
//! so each scenario exercises handler responses travelling through the
//! connection actor rather than a hand-rolled transport.

use std::{pin::Pin, sync::Arc};

use bytes::Bytes;
use futures::StreamExt;
use rstest::fixture;
use tokio::sync::mpsc;
use wireframe::{
    app::{Envelope, ResponseHandler, WireframeApp},
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

use crate::fixtures::client_streaming::{MessageId, Payload, StreamTestEnvelope, TERMINATOR_ID};

/// Route identifier for a lazily streamed response.
const STREAM_ROUTE: u32 = 20;
/// Route identifier for a spawned multi-packet response.
const MULTI_ROUTE: u32 = 21;
/// Route identifier for an empty streamed response.
const EMPTY_ROUTE: u32 = 22;
/// Maximum frame length shared by the paired client and server.
const MAX_FRAME_LENGTH: usize = 2048;

/// Test world for server-side streaming scenarios.
pub struct ServerStreamingWorld {
    runtime: Option<tokio::runtime::Runtime>,
    runtime_error: Option<String>,
    pair: Option<WireframePair>,
    received: Vec<StreamTestEnvelope>,
    last_error: Option<TestError>,
    terminated_cleanly: bool,
}

impl std::fmt::Debug for ServerStreamingWorld {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ServerStreamingWorld")
            .field("received", &self.received.len())
            .field("has_error", &self.last_error.is_some())
            .finish_non_exhaustive()
    }
}

/// Fixture for `ServerStreamingWorld`.
///
/// The body stays on its own line because the single-line form makes the
/// `#[fixture]` expansion trip `unused_braces`; `rustfmt` would otherwise
/// collapse it.
#[fixture]
#[rustfmt::skip]
pub fn server_streaming_world() -> ServerStreamingWorld {
    ServerStreamingWorld::new()
}

impl ServerStreamingWorld {
    /// Build a world with a fresh current-thread runtime.
    fn new() -> Self {
        let (runtime, runtime_error) = match tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
        {
            Ok(runtime) => (Some(runtime), None),
            Err(error) => (None, Some(format!("failed to create runtime: {error}"))),
        };
        Self {
            runtime,
            runtime_error,
            pair: None,
            received: Vec::new(),
            last_error: None,
            terminated_cleanly: false,
        }
    }

    /// Run a future on the shared runtime while yielding world ownership.
    pub fn block_on<T>(
        &mut self,
        f: impl for<'a> FnOnce(&'a mut Self) -> Pin<Box<dyn std::future::Future<Output = T> + 'a>>,
    ) -> TestResult<T> {
        let error = self
            .runtime_error
            .as_deref()
            .unwrap_or("runtime unavailable");
        let runtime = self.runtime.take().ok_or(error)?;
        let result = runtime.block_on(f(self));
        self.runtime = Some(runtime);
        Ok(result)
    }

    /// Start the paired server and client.
    pub async fn start_server(&mut self) -> TestResult {
        self.received.clear();
        self.last_error = None;
        self.terminated_cleanly = false;
        let pair = spawn_wireframe_pair(build_app, |builder| {
            builder.max_frame_length(MAX_FRAME_LENGTH)
        })
        .await?;
        self.pair = Some(pair);
        Ok(())
    }

    /// Request one of the streaming routes and collect the response.
    pub async fn request_route(&mut self, route: u32) -> TestResult {
        self.received.clear();
        self.last_error = None;
        self.terminated_cleanly = false;
        let pair = self.pair.as_mut().ok_or("server not started")?;
        let request = StreamTestEnvelope {
            id: MessageId::new(route),
            correlation_id: None,
            payload: Payload::new(Vec::new()),
        };
        let client = pair.client_mut()?;
        let result = collect_stream(client, request).await;
        match result {
            Ok(frames) => {
                self.received = frames;
                self.terminated_cleanly = true;
            }
            Err(error) => self.last_error = Some(error),
        }
        Ok(())
    }

    /// Assert the response stream ended at its protocol terminator.
    pub fn verify_clean_termination(&self) -> TestResult {
        if let Some(error) = &self.last_error {
            return Err(TestError::Msg(format!("stream failed: {error}")));
        }
        if !self.terminated_cleanly {
            return Err(TestError::Msg(String::from(
                "the response stream did not reach its terminator",
            )));
        }
        Ok(())
    }

    /// Assert the number of data frames received before the terminator.
    pub fn verify_frame_count(&self, expected: usize) -> TestResult {
        if let Some(error) = &self.last_error {
            return Err(TestError::Msg(format!("stream failed: {error}")));
        }
        if self.received.len() != expected {
            return Err(TestError::Msg(format!(
                "expected {expected} frames, received {}",
                self.received.len()
            )));
        }
        Ok(())
    }

    /// Assert the payload bytes arrive in ascending order starting at one.
    pub fn verify_frame_order(&self) -> TestResult {
        for (index, frame) in self.received.iter().enumerate() {
            let expected = u8::try_from(index + 1)
                .map_err(|error| TestError::Msg(format!("frame index overflow: {error}")))?;
            let payload = frame.payload.clone().into_inner();
            if payload != vec![expected] {
                return Err(TestError::Msg(format!(
                    "frame {index} carried {payload:?}, expected [{expected}]"
                )));
            }
        }
        Ok(())
    }
}

/// Build the application whose routes return explicit responses.
fn build_app() -> wireframe::Result<WireframeApp<BincodeSerializer, (), StreamTestEnvelope>> {
    Ok(
        WireframeApp::<BincodeSerializer, (), StreamTestEnvelope>::default()
            .route_response(STREAM_ROUTE, stream_handler())?
            .route_response(MULTI_ROUTE, multi_handler())?
            .route_response(EMPTY_ROUTE, empty_handler())?
            .with_protocol(TerminatingProtocol),
    )
}

/// Handler that lazily yields three data frames.
fn stream_handler() -> ResponseHandler<StreamTestEnvelope> {
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
            .send(Envelope::new(2, None, vec![payload]))
            .await
            .is_err()
        {
            return;
        }
    }
}

/// Handler that streams two frames from a spawned producer.
fn multi_handler() -> ResponseHandler<StreamTestEnvelope> {
    Arc::new(|_request: &StreamTestEnvelope| {
        Box::pin(async {
            let (sender, response) = Response::with_channel(2);
            tokio::spawn(send_payloads(sender, [1_u8, 2]));
            response
        })
    })
}

/// Handler that returns an empty stream, so only the terminator is sent.
fn empty_handler() -> ResponseHandler<StreamTestEnvelope> {
    Arc::new(|_request: &StreamTestEnvelope| {
        Box::pin(async {
            let stream: FrameStream<Envelope, ()> = Box::pin(futures::stream::empty());
            Response::Stream(stream)
        })
    })
}

/// Protocol that terminates every stream with an id-zero envelope.
struct TerminatingProtocol;

impl WireframeProtocol for TerminatingProtocol {
    type Frame = Bytes;
    type ProtocolError = ();

    fn stream_end_frame(&self, _ctx: &mut ConnectionContext) -> Option<Self::Frame> {
        let envelope = Envelope::new(TERMINATOR_ID.get(), None, Vec::new());
        // An unencodable terminator is treated as "no terminator"; the fixture
        // envelope always encodes.
        let payload = BincodeSerializer.serialize(&envelope).ok()?;
        Some(LengthDelimitedFrameCodec::new(MAX_FRAME_LENGTH).wrap_payload(Bytes::from(payload)))
    }
}

/// Send `request` and collect data frames until the terminator arrives.
async fn collect_stream(
    client: &mut wireframe::client::WireframeClient<
        BincodeSerializer,
        wireframe::rewind_stream::RewindStream<tokio::net::TcpStream>,
    >,
    request: StreamTestEnvelope,
) -> TestResult<Vec<StreamTestEnvelope>> {
    let mut stream = client.call_streaming::<StreamTestEnvelope>(request).await?;
    let mut frames = Vec::new();
    while let Some(frame) = stream.next().await {
        frames.push(frame?);
    }
    Ok(frames)
}

/// Route identifier requested by the streamed scenario.
pub const STREAMED_ROUTE: u32 = STREAM_ROUTE;
/// Route identifier requested by the multi-packet scenario.
pub const MULTI_PACKET_ROUTE: u32 = MULTI_ROUTE;
/// Route identifier requested by the empty-stream scenario.
pub const EMPTY_STREAM_ROUTE: u32 = EMPTY_ROUTE;
