//! Shared test infrastructure for client streaming response tests.
//!
//! Contains newtypes, the `TestStreamEnvelope` helper, `TestServer`, and
//! server/client factory functions used by the streaming test module.

use std::net::SocketAddr;

use bytes::Bytes;
use derive_more::{Display, From};
use futures::{SinkExt, StreamExt};
use rstest::fixture;
use tokio::net::TcpListener;
use tokio_util::codec::{Framed, LengthDelimitedCodec};

use crate::{
    BincodeSerializer,
    Serializer,
    WireframeClient,
    app::Packet,
    correlation::CorrelatableFrame,
    rewind_stream::RewindStream,
};

// ---------------------------------------------------------------------------
// Newtypes — eliminate integer soup
// ---------------------------------------------------------------------------

/// A correlation identifier used to match streaming requests to responses.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Display, From)]
#[display("{_0}")]
pub(super) struct CorrelationId(u64);

impl CorrelationId {
    pub(super) const fn new(value: u64) -> Self { Self(value) }
    pub(super) const fn get(self) -> u64 { self.0 }
}

impl From<CorrelationId> for u64 {
    fn from(value: CorrelationId) -> Self { value.0 }
}

/// A message identifier used for routing.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Display, From)]
#[display("{_0}")]
pub(super) struct MessageId(u32);

impl MessageId {
    pub(super) const fn new(value: u32) -> Self { Self(value) }
    pub(super) const fn get(self) -> u32 { self.0 }
}

impl From<MessageId> for u32 {
    fn from(value: MessageId) -> Self { value.0 }
}

/// Payload bytes carried by a test frame.
#[derive(Clone, Debug, PartialEq, Eq, From)]
pub(super) struct Payload(Vec<u8>);

impl Payload {
    pub(super) fn new(data: Vec<u8>) -> Self { Self(data) }
    pub(super) fn into_inner(self) -> Vec<u8> { self.0 }
}

impl AsRef<[u8]> for Payload {
    fn as_ref(&self) -> &[u8] { &self.0 }
}

// ---------------------------------------------------------------------------
// TestStreamEnvelope
// ---------------------------------------------------------------------------

/// Terminator message ID used by the test protocol.
pub(super) const TERMINATOR_ID: MessageId = MessageId::new(0);

/// A test envelope that treats `id == 0` as a stream terminator.
#[derive(bincode::Decode, bincode::Encode, Debug, Clone, PartialEq, Eq)]
pub(super) struct TestStreamEnvelope {
    pub id: u32,
    pub correlation_id: Option<u64>,
    pub payload: Vec<u8>,
}

impl CorrelatableFrame for TestStreamEnvelope {
    fn correlation_id(&self) -> Option<u64> { self.correlation_id }

    fn set_correlation_id(&mut self, cid: Option<u64>) { self.correlation_id = cid; }
}

impl Packet for TestStreamEnvelope {
    fn id(&self) -> u32 { self.id }

    fn into_parts(self) -> crate::app::PacketParts {
        crate::app::PacketParts::new(self.id, self.correlation_id, self.payload)
    }

    fn from_parts(parts: crate::app::PacketParts) -> Self {
        Self {
            id: parts.id(),
            correlation_id: parts.correlation_id(),
            payload: parts.into_payload(),
        }
    }

    fn is_stream_terminator(&self) -> bool { self.id == TERMINATOR_ID.get() }
}

impl TestStreamEnvelope {
    pub(super) fn data(id: MessageId, correlation_id: CorrelationId, payload: Payload) -> Self {
        Self {
            id: id.get(),
            correlation_id: Some(correlation_id.get()),
            payload: payload.into_inner(),
        }
    }

    pub(super) fn terminator(correlation_id: CorrelationId) -> Self {
        Self {
            id: TERMINATOR_ID.get(),
            correlation_id: Some(correlation_id.get()),
            payload: vec![],
        }
    }
}

// ---------------------------------------------------------------------------
// Test infrastructure
// ---------------------------------------------------------------------------

/// Convenience alias for the test client type.
pub(super) type TestClient =
    WireframeClient<BincodeSerializer, RewindStream<tokio::net::TcpStream>>;

/// Context for a streaming test: the server address and a handle to join the
/// server task. Automatically aborts the server on drop.
pub(super) struct TestServer {
    pub addr: SocketAddr,
    handle: tokio::task::JoinHandle<()>,
}

impl TestServer {
    /// Construct a `TestServer` from a pre-spawned task handle.
    pub(super) fn from_handle(addr: SocketAddr, handle: tokio::task::JoinHandle<()>) -> Self {
        Self { addr, handle }
    }
}

impl Drop for TestServer {
    fn drop(&mut self) { self.handle.abort(); }
}

/// Serialize a `TestStreamEnvelope` to bytes for transmission.
///
/// # Errors
/// Returns an error if serialization fails.
fn serialize_envelope(
    envelope: &TestStreamEnvelope,
) -> Result<Bytes, Box<dyn std::error::Error + Send + Sync>> {
    Ok(Bytes::from(BincodeSerializer.serialize(envelope)?))
}

/// Spawn a test server that sends a sequence of pre-built frames.
///
/// When `close_without_terminator` is `true`, the connection is dropped after
/// sending the frames (simulating an unexpected disconnect). When `false`, the
/// frames are sent as-is — the caller is responsible for including a
/// terminator in the `frames` vec when normal termination is desired.
///
/// # Errors
/// Returns an error if the TCP listener cannot be bound.
pub(super) async fn spawn_test_server(
    frames: Vec<TestStreamEnvelope>,
    close_without_terminator: bool,
) -> Result<TestServer, Box<dyn std::error::Error + Send + Sync>> {
    let listener = TcpListener::bind("127.0.0.1:0").await?;
    let addr = listener.local_addr()?;

    let handle = tokio::spawn(async move {
        let Ok((tcp, _)) = listener.accept().await else {
            return;
        };
        let mut transport = Framed::new(tcp, LengthDelimitedCodec::new());

        // Read one request frame (the client's request), then send the
        // pre-built sequence.
        let _request = transport.next().await;

        for frame in &frames {
            let Ok(encoded) = serialize_envelope(frame) else {
                break;
            };
            if transport.send(encoded).await.is_err() {
                break;
            }
        }

        if close_without_terminator {
            // Connection is dropped here — no terminator sent.
        }
    });

    Ok(TestServer { addr, handle })
}

/// Spawn a test server that sends a frame with a mismatched correlation ID.
///
/// # Errors
/// Returns an error if the TCP listener cannot be bound.
pub(super) async fn spawn_mismatch_server(
    wrong_correlation_id: CorrelationId,
) -> Result<TestServer, Box<dyn std::error::Error + Send + Sync>> {
    let listener = TcpListener::bind("127.0.0.1:0").await?;
    let addr = listener.local_addr()?;

    let handle = tokio::spawn(async move {
        let Ok((tcp, _)) = listener.accept().await else {
            return;
        };
        let mut transport = Framed::new(tcp, LengthDelimitedCodec::new());

        let _request = transport.next().await;

        let bad_frame = TestStreamEnvelope::data(
            MessageId::new(1),
            wrong_correlation_id,
            Payload::new(vec![99]),
        );
        let Ok(encoded) = serialize_envelope(&bad_frame) else {
            return;
        };
        let _ = transport.send(encoded).await;
    });

    Ok(TestServer { addr, handle })
}

/// Spawn a test server that sends invalid bytes (for decode-error testing).
///
/// # Errors
/// Returns an error if the TCP listener cannot be bound.
pub(super) async fn spawn_malformed_server()
-> Result<TestServer, Box<dyn std::error::Error + Send + Sync>> {
    let listener = TcpListener::bind("127.0.0.1:0").await?;
    let addr = listener.local_addr()?;

    let handle = tokio::spawn(async move {
        let Ok((tcp, _)) = listener.accept().await else {
            return;
        };
        let mut transport = Framed::new(tcp, LengthDelimitedCodec::new());

        // Read request.
        let _request = transport.next().await;

        // Send truncated/invalid bytes that cannot be decoded.
        let invalid_bytes = Bytes::from_static(b"\xff\xff\xff");
        let _ = transport.send(invalid_bytes).await;
    });

    Ok(TestServer { addr, handle })
}

/// Create a connected `WireframeClient` for the given address.
///
/// # Errors
/// Returns an error if the connection fails.
pub(super) async fn create_test_client(
    addr: SocketAddr,
) -> Result<TestClient, Box<dyn std::error::Error + Send + Sync>> {
    Ok(WireframeClient::builder().connect(addr).await?)
}

/// Spawn a test server with the given frames and return a connected client.
///
/// # Errors
/// Returns an error if server setup or client connection fails.
pub(super) async fn setup_streaming_test(
    frames: Vec<TestStreamEnvelope>,
) -> Result<(TestClient, TestServer), Box<dyn std::error::Error + Send + Sync>> {
    let server = spawn_test_server(frames, false).await?;
    let client = create_test_client(server.addr).await?;
    Ok((client, server))
}

// ---------------------------------------------------------------------------
// Fixtures
// ---------------------------------------------------------------------------

/// Fixed test correlation identifier shared by all streaming tests.
#[rustfmt::skip]
#[fixture]
pub(super) fn correlation_id() -> CorrelationId {
    CorrelationId::new(42)
}
