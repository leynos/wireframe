//! `ClientStreamingWorld` fixture for rstest-bdd tests.
//!
//! Provides server/client coordination for streaming response consumption
//! tests. The test server sends a configurable sequence of correlated frames
//! followed by a terminator, and the world drives the client's
//! `call_streaming` API.

use std::net::SocketAddr;

use bytes::Bytes;
use futures::{SinkExt, StreamExt};
use rstest::fixture;
use tokio::{net::TcpListener, task::JoinHandle};
use tokio_util::codec::{Framed, LengthDelimitedCodec};
use wireframe::{
    BincodeSerializer,
    Serializer,
    app::{Packet, PacketParts},
    client::{ClientError, WireframeClient},
    correlation::CorrelatableFrame,
    rewind_stream::RewindStream,
};
pub use wireframe_testing::TestResult;

/// Terminator message ID used by the test streaming protocol.
const TERMINATOR_ID: u32 = 0;

/// A test envelope that treats `id == 0` as a stream terminator.
#[derive(bincode::Decode, bincode::Encode, Debug, Clone, PartialEq, Eq)]
pub struct StreamTestEnvelope {
    pub id: u32,
    pub correlation_id: Option<u64>,
    pub payload: Vec<u8>,
}

impl CorrelatableFrame for StreamTestEnvelope {
    fn correlation_id(&self) -> Option<u64> { self.correlation_id }

    fn set_correlation_id(&mut self, cid: Option<u64>) { self.correlation_id = cid; }
}

impl Packet for StreamTestEnvelope {
    fn id(&self) -> u32 { self.id }

    fn into_parts(self) -> PacketParts {
        PacketParts::new(self.id, self.correlation_id, self.payload)
    }

    fn from_parts(parts: PacketParts) -> Self {
        Self {
            id: parts.id(),
            correlation_id: parts.correlation_id(),
            payload: parts.into_payload(),
        }
    }

    fn is_stream_terminator(&self) -> bool { self.id == TERMINATOR_ID }
}

impl StreamTestEnvelope {
    fn data(id: u32, correlation_id: u64, payload: Vec<u8>) -> Self {
        Self {
            id,
            correlation_id: Some(correlation_id),
            payload,
        }
    }

    fn terminator(correlation_id: u64) -> Self {
        Self {
            id: TERMINATOR_ID,
            correlation_id: Some(correlation_id),
            payload: vec![],
        }
    }

    /// Serialize this envelope to bytes for transmission.
    ///
    /// # Errors
    /// Returns an error if serialization fails.
    fn serialize_to_bytes(&self) -> Result<Bytes, Box<dyn std::error::Error + Send + Sync>> {
        Ok(Bytes::from(BincodeSerializer.serialize(self)?))
    }
}

/// Mode controlling how the streaming test server behaves.
enum StreamingServerMode {
    /// Send `data_count` data frames then a terminator.
    Normal { data_count: usize },
    /// Send one frame with a wrong correlation ID.
    Mismatch,
    /// Send `data_count` data frames then drop the connection.
    Disconnect { data_count: usize },
}

/// Test world for client streaming scenarios.
#[derive(Default)]
pub struct ClientStreamingWorld {
    addr: Option<SocketAddr>,
    server: Option<JoinHandle<()>>,
    client: Option<WireframeClient<BincodeSerializer, RewindStream<tokio::net::TcpStream>>>,
    received_frames: Vec<StreamTestEnvelope>,
    last_error: Option<ClientError>,
    stream_terminated_cleanly: bool,
}

impl std::fmt::Debug for ClientStreamingWorld {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ClientStreamingWorld")
            .field("addr", &self.addr)
            .field("received_frames", &self.received_frames.len())
            .field("stream_terminated_cleanly", &self.stream_terminated_cleanly)
            .finish_non_exhaustive()
    }
}

/// Fixture for `ClientStreamingWorld`.
#[rustfmt::skip]
#[fixture]
pub fn client_streaming_world() -> ClientStreamingWorld {
    ClientStreamingWorld::default()
}

impl ClientStreamingWorld {
    /// Start a streaming server that sends `data_count` frames + terminator.
    ///
    /// # Errors
    /// Returns an error if binding or spawning fails.
    pub async fn start_normal_server(&mut self, data_count: usize) -> TestResult {
        self.start_server(StreamingServerMode::Normal { data_count })
            .await
    }

    /// Start a streaming server that returns a mismatched correlation ID.
    ///
    /// # Errors
    /// Returns an error if binding or spawning fails.
    pub async fn start_mismatch_server(&mut self) -> TestResult {
        self.start_server(StreamingServerMode::Mismatch).await
    }

    /// Start a server that sends `data_count` frames then disconnects.
    ///
    /// # Errors
    /// Returns an error if binding or spawning fails.
    pub async fn start_disconnect_server(&mut self, data_count: usize) -> TestResult {
        self.start_server(StreamingServerMode::Disconnect { data_count })
            .await
    }

    async fn start_server(&mut self, mode: StreamingServerMode) -> TestResult {
        let listener = TcpListener::bind("127.0.0.1:0").await?;
        let addr = listener.local_addr()?;

        let handle = tokio::spawn(async move {
            let Ok((stream, _)) = listener.accept().await else {
                return;
            };
            let mut framed = Framed::new(stream, LengthDelimitedCodec::new());

            // Read the client's request to extract the correlation ID.
            let Some(Ok(req_bytes)) = framed.next().await else {
                return;
            };
            let Ok((req, _)): Result<(StreamTestEnvelope, usize), _> =
                BincodeSerializer.deserialize(&req_bytes)
            else {
                return;
            };
            let cid = req.correlation_id().unwrap_or(1);

            match mode {
                StreamingServerMode::Normal { data_count } => {
                    send_data_and_terminator(&mut framed, cid, data_count).await;
                }
                StreamingServerMode::Mismatch => {
                    send_mismatch_frame(&mut framed, cid).await;
                }
                StreamingServerMode::Disconnect { data_count } => {
                    send_data_frames(&mut framed, cid, data_count).await;
                    // Drop without terminator.
                }
            }
        });

        self.addr = Some(addr);
        self.server = Some(handle);
        Ok(())
    }

    /// Connect a client to the server.
    ///
    /// # Errors
    /// Returns an error if the server is not started or connection fails.
    pub async fn connect_client(&mut self) -> TestResult {
        let addr = self.addr.ok_or("server address missing")?;
        let client = WireframeClient::builder().connect(addr).await?;
        self.client = Some(client);
        Ok(())
    }

    /// Send a streaming request and consume the response stream.
    ///
    /// # Errors
    /// Returns an error if client is missing or request send fails.
    pub async fn send_streaming_request(&mut self) -> TestResult {
        let client = self.client.as_mut().ok_or("client not connected")?;

        let request = StreamTestEnvelope {
            id: 99,
            correlation_id: None,
            payload: vec![],
        };

        let mut stream = client.call_streaming::<StreamTestEnvelope>(request).await?;

        self.received_frames.clear();
        self.last_error = None;
        self.stream_terminated_cleanly = false;

        loop {
            match stream.next().await {
                Some(Ok(frame)) => {
                    self.received_frames.push(frame);
                }
                Some(Err(e)) => {
                    self.last_error = Some(e);
                    break;
                }
                None => {
                    self.stream_terminated_cleanly = true;
                    break;
                }
            }
        }

        Ok(())
    }

    /// Verify the count of received data frames.
    ///
    /// # Errors
    /// Returns an error if the count does not match.
    pub fn verify_frame_count(&self, expected: usize) -> TestResult {
        let actual = self.received_frames.len();
        if actual != expected {
            return Err(format!("expected {expected} frames, got {actual}").into());
        }
        Ok(())
    }

    /// Verify frames arrived in order (payload == [1], [2], ...).
    ///
    /// # Errors
    /// Returns an error if ordering is wrong.
    pub fn verify_frame_order(&self) -> TestResult {
        for (i, frame) in self.received_frames.iter().enumerate() {
            let payload_byte =
                u8::try_from(i + 1).map_err(|e| format!("frame index {i} overflows u8: {e}"))?;
            let expected_payload = vec![payload_byte];
            if frame.payload != expected_payload {
                return Err(format!(
                    "frame {i}: expected payload {expected_payload:?}, got {:?}",
                    frame.payload
                )
                .into());
            }
        }
        Ok(())
    }

    /// Verify the stream terminated cleanly (received None).
    ///
    /// # Errors
    /// Returns an error if the stream did not terminate cleanly.
    pub fn verify_clean_termination(&self) -> TestResult {
        if !self.stream_terminated_cleanly {
            return Err("stream did not terminate cleanly".into());
        }
        Ok(())
    }

    /// Verify that a `StreamCorrelationMismatch` error was returned.
    ///
    /// # Errors
    /// Returns an error if a different error or no error occurred.
    pub fn verify_correlation_mismatch_error(&self) -> TestResult {
        match &self.last_error {
            Some(ClientError::StreamCorrelationMismatch { .. }) => Ok(()),
            Some(err) => Err(format!("expected StreamCorrelationMismatch, got {err:?}").into()),
            None => Err("expected StreamCorrelationMismatch, but no error".into()),
        }
    }

    /// Verify that a transport/disconnect error was returned.
    ///
    /// # Errors
    /// Returns an error if no transport error occurred.
    pub fn verify_disconnect_error(&self) -> TestResult {
        match &self.last_error {
            Some(ClientError::Wireframe(_)) => Ok(()),
            Some(err) => Err(format!("expected transport error, got {err:?}").into()),
            None => Err("expected transport error, but no error".into()),
        }
    }

    /// Abort the server task.
    pub fn abort_server(&mut self) {
        if let Some(handle) = self.server.take() {
            handle.abort();
        }
    }
}

/// Send a single frame with a mismatched correlation ID.
async fn send_mismatch_frame<T>(framed: &mut Framed<T, LengthDelimitedCodec>, cid: u64)
where
    T: tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin,
{
    let bad = StreamTestEnvelope::data(1, cid + 999, vec![99]);
    if let Ok(encoded) = bad.serialize_to_bytes() {
        let _ = framed.send(encoded).await;
    }
}

/// Send `count` data frames with payload `[1], [2], ..., [count]`.
async fn send_data_frames<T>(framed: &mut Framed<T, LengthDelimitedCodec>, cid: u64, count: usize)
where
    T: tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin,
{
    for i in 1..=count {
        let Ok(id) = u32::try_from(i) else { break };
        let Ok(payload_byte) = u8::try_from(i) else {
            break;
        };
        let frame = StreamTestEnvelope::data(id, cid, vec![payload_byte]);
        let Ok(encoded) = frame.serialize_to_bytes() else {
            break;
        };
        if framed.send(encoded).await.is_err() {
            break;
        }
    }
}

/// Send `count` data frames followed by a terminator.
async fn send_data_and_terminator<T>(
    framed: &mut Framed<T, LengthDelimitedCodec>,
    cid: u64,
    count: usize,
) where
    T: tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin,
{
    send_data_frames(framed, cid, count).await;
    let term = StreamTestEnvelope::terminator(cid);
    if let Ok(encoded) = term.serialize_to_bytes() {
        let _ = framed.send(encoded).await;
    }
}
