//! Unit tests for client streaming response APIs.

use std::net::SocketAddr;

use bytes::Bytes;
use futures::{SinkExt, StreamExt};
use rstest::{fixture, rstest};
use tokio::net::TcpListener;
use tokio_util::codec::{Framed, LengthDelimitedCodec};

use crate::{
    BincodeSerializer,
    Serializer,
    WireframeClient,
    app::Packet,
    client::ClientError,
    correlation::CorrelatableFrame,
};

// Type aliases to reduce primitive obsession
type CorrelationId = u64;
type MessageId = u32;
type Payload = Vec<u8>;

/// Terminator message ID used by the test protocol.
const TERMINATOR_ID: MessageId = 0;

/// A test envelope that treats `id == 0` as a stream terminator.
#[derive(bincode::Decode, bincode::Encode, Debug, Clone, PartialEq, Eq)]
struct TestStreamEnvelope {
    id: u32,
    correlation_id: Option<u64>,
    payload: Vec<u8>,
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

    fn is_stream_terminator(&self) -> bool { self.id == TERMINATOR_ID }
}

impl TestStreamEnvelope {
    fn data(id: MessageId, correlation_id: CorrelationId, payload: Payload) -> Self {
        Self {
            id,
            correlation_id: Some(correlation_id),
            payload,
        }
    }

    fn terminator(correlation_id: CorrelationId) -> Self {
        Self {
            id: TERMINATOR_ID,
            correlation_id: Some(correlation_id),
            payload: vec![],
        }
    }
}

/// Serializes a `TestStreamEnvelope` to bytes for transmission.
fn serialize_envelope(envelope: &TestStreamEnvelope) -> Bytes {
    Bytes::from(
        BincodeSerializer
            .serialize(envelope)
            .expect("serialize test envelope"),
    )
}

// ---------------------------------------------------------------------------
// Test infrastructure
// ---------------------------------------------------------------------------

/// Context for a streaming test: the server address and a handle to join the
/// server task. Automatically aborts the server on drop.
struct TestServer {
    addr: SocketAddr,
    handle: tokio::task::JoinHandle<()>,
}

impl Drop for TestServer {
    fn drop(&mut self) { self.handle.abort(); }
}

/// Spawn a test server that sends a sequence of pre-built frames.
///
/// When `close_without_terminator` is `true`, the connection is dropped after
/// sending the frames (simulating an unexpected disconnect). When `false`, the
/// frames are sent as-is — the caller is responsible for including a
/// terminator in the `frames` vec when normal termination is desired.
async fn spawn_test_server(
    frames: Vec<TestStreamEnvelope>,
    close_without_terminator: bool,
) -> TestServer {
    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind listener");
    let addr = listener.local_addr().expect("listener addr");

    let handle = tokio::spawn(async move {
        let (tcp, _) = listener.accept().await.expect("accept client");
        let mut transport = Framed::new(tcp, LengthDelimitedCodec::new());

        // Read one request frame (the client's request), then send the
        // pre-built sequence.
        let _request = transport.next().await;

        for frame in &frames {
            let encoded = serialize_envelope(frame);
            if transport.send(encoded).await.is_err() {
                break;
            }
        }

        if close_without_terminator {
            // Connection is dropped here — no terminator sent.
        }
    });

    TestServer { addr, handle }
}

/// Spawn a test server that sends a frame with a mismatched correlation ID.
async fn spawn_mismatch_server(wrong_correlation_id: CorrelationId) -> TestServer {
    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind listener");
    let addr = listener.local_addr().expect("listener addr");

    let handle = tokio::spawn(async move {
        let (tcp, _) = listener.accept().await.expect("accept client");
        let mut transport = Framed::new(tcp, LengthDelimitedCodec::new());

        let _request = transport.next().await;

        let bad_frame = TestStreamEnvelope::data(1, wrong_correlation_id, vec![99]);
        let encoded = serialize_envelope(&bad_frame);
        let _ = transport.send(encoded).await;
    });

    TestServer { addr, handle }
}

/// Create a connected `WireframeClient` for the given address.
async fn create_test_client(
    addr: SocketAddr,
) -> WireframeClient<BincodeSerializer, crate::rewind_stream::RewindStream<tokio::net::TcpStream>> {
    WireframeClient::builder()
        .connect(addr)
        .await
        .expect("connect client")
}

/// Spawn a test server with the given frames and return a connected client.
async fn setup_streaming_test(
    frames: Vec<TestStreamEnvelope>,
) -> (
    WireframeClient<BincodeSerializer, crate::rewind_stream::RewindStream<tokio::net::TcpStream>>,
    TestServer,
) {
    let server = spawn_test_server(frames, false).await;
    let client = create_test_client(server.addr).await;
    (client, server)
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[fixture]
fn correlation_id() -> CorrelationId {
    // Fixed test correlation identifier shared by all streaming tests.
    42
}

#[rstest]
#[tokio::test]
async fn response_stream_yields_data_frames_in_order(correlation_id: u64) {
    let frames = vec![
        TestStreamEnvelope::data(1, correlation_id, vec![10]),
        TestStreamEnvelope::data(2, correlation_id, vec![20]),
        TestStreamEnvelope::data(3, correlation_id, vec![30]),
        TestStreamEnvelope::terminator(correlation_id),
    ];

    let (mut client, _server) = setup_streaming_test(frames).await;

    let request = TestStreamEnvelope::data(99, correlation_id, vec![]);
    let mut stream = client
        .call_streaming::<TestStreamEnvelope>(request)
        .await
        .expect("call_streaming should succeed");

    let mut received = Vec::new();
    while let Some(result) = stream.next().await {
        received.push(result.expect("data frame should be Ok"));
    }

    assert_eq!(received.len(), 3, "should receive exactly 3 data frames");
    assert_eq!(received.first().expect("frame 0").payload, vec![10]);
    assert_eq!(received.get(1).expect("frame 1").payload, vec![20]);
    assert_eq!(received.get(2).expect("frame 2").payload, vec![30]);
}

#[rstest]
#[tokio::test]
async fn response_stream_terminates_on_terminator(correlation_id: u64) {
    let frames = vec![
        TestStreamEnvelope::data(1, correlation_id, vec![1]),
        TestStreamEnvelope::terminator(correlation_id),
    ];

    let (mut client, _server) = setup_streaming_test(frames).await;

    let request = TestStreamEnvelope::data(99, correlation_id, vec![]);
    let mut stream = client
        .call_streaming::<TestStreamEnvelope>(request)
        .await
        .expect("call_streaming");

    // First item is the data frame.
    let first = stream.next().await;
    assert!(first.is_some(), "should yield one data frame");
    assert!(first.expect("some").is_ok(), "data frame should be Ok");

    // Second poll returns None (terminator consumed).
    let second = stream.next().await;
    assert!(second.is_none(), "stream should terminate after terminator");

    // Subsequent polls also return None.
    let third = stream.next().await;
    assert!(third.is_none(), "stream should remain terminated");

    assert!(stream.is_terminated(), "is_terminated should be true");
}

#[rstest]
#[tokio::test]
async fn response_stream_validates_correlation_id(correlation_id: u64) {
    let wrong_cid = correlation_id + 999;
    let server = spawn_mismatch_server(wrong_cid).await;
    let mut client = create_test_client(server.addr).await;

    let mut request = TestStreamEnvelope::data(99, correlation_id, vec![]);
    request.set_correlation_id(Some(correlation_id));

    let mut stream = client
        .call_streaming::<TestStreamEnvelope>(request)
        .await
        .expect("call_streaming");

    let result = stream.next().await;
    match result {
        Some(Err(ClientError::StreamCorrelationMismatch { expected, received })) => {
            assert_eq!(expected, Some(correlation_id));
            assert_eq!(received, Some(wrong_cid));
        }
        other => panic!("expected StreamCorrelationMismatch, got {other:?}"),
    }
}

#[rstest]
#[tokio::test]
async fn response_stream_handles_empty_stream(correlation_id: u64) {
    let frames = vec![TestStreamEnvelope::terminator(correlation_id)];

    let (mut client, _server) = setup_streaming_test(frames).await;

    let request = TestStreamEnvelope::data(99, correlation_id, vec![]);
    let mut stream = client
        .call_streaming::<TestStreamEnvelope>(request)
        .await
        .expect("call_streaming");

    // Stream should immediately return None (only terminator was sent).
    let first = stream.next().await;
    assert!(
        first.is_none(),
        "empty stream should yield None immediately"
    );
}

#[rstest]
#[tokio::test]
async fn response_stream_handles_connection_close(correlation_id: u64) {
    let frames = vec![
        TestStreamEnvelope::data(1, correlation_id, vec![10]),
        TestStreamEnvelope::data(2, correlation_id, vec![20]),
    ];

    let server = spawn_test_server(frames, true).await;
    let mut client = create_test_client(server.addr).await;

    let request = TestStreamEnvelope::data(99, correlation_id, vec![]);
    let mut stream = client
        .call_streaming::<TestStreamEnvelope>(request)
        .await
        .expect("call_streaming");

    // Should receive the two data frames.
    let first = stream.next().await.expect("first frame").expect("Ok");
    assert_eq!(first.payload, vec![10]);
    let second = stream.next().await.expect("second frame").expect("Ok");
    assert_eq!(second.payload, vec![20]);

    // Next poll should return an error (connection closed without terminator).
    let third = stream.next().await;
    assert!(
        matches!(third, Some(Err(ClientError::Wireframe(_)))),
        "should return a transport error on disconnect, got {third:?}"
    );
}

#[rstest]
#[tokio::test]
async fn call_streaming_sends_request_and_returns_stream(correlation_id: u64) {
    let frames = vec![
        TestStreamEnvelope::data(1, correlation_id, vec![77]),
        TestStreamEnvelope::terminator(correlation_id),
    ];

    let (mut client, _server) = setup_streaming_test(frames).await;

    // Use explicit correlation ID so server response matches.
    let request = TestStreamEnvelope::data(99, correlation_id, vec![]);
    let mut stream = client
        .call_streaming::<TestStreamEnvelope>(request)
        .await
        .expect("call_streaming");

    assert_eq!(stream.correlation_id(), correlation_id);

    let frame = stream.next().await.expect("one data frame").expect("Ok");
    assert_eq!(frame.payload, vec![77]);

    let end = stream.next().await;
    assert!(end.is_none(), "stream should terminate");
}

#[rstest]
#[tokio::test]
async fn call_streaming_auto_generates_correlation_id() {
    // This server echoes frames with matching CID, so we need a smarter
    // server that captures the request's CID and uses it in the response.
    let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind");
    let addr = listener.local_addr().expect("addr");

    let _server = TestServer {
        addr,
        handle: tokio::spawn(async move {
            let (tcp, _) = listener.accept().await.expect("accept");
            let mut transport = Framed::new(tcp, LengthDelimitedCodec::new());

            // Read request, extract correlation ID.
            if let Some(Ok(req_bytes)) = transport.next().await {
                let (req, _): (TestStreamEnvelope, usize) = BincodeSerializer
                    .deserialize(&req_bytes)
                    .expect("deser request");
                let cid = req.correlation_id().expect("request should have CID");

                // Send one data frame + terminator with matching CID.
                let data = TestStreamEnvelope::data(1, cid, vec![42]);
                let _ = transport.send(serialize_envelope(&data)).await;

                let term = TestStreamEnvelope::terminator(cid);
                let _ = transport.send(serialize_envelope(&term)).await;
            }
        }),
    };

    let mut client = create_test_client(addr).await;

    // Send request without explicit correlation ID.
    let request = TestStreamEnvelope {
        id: 99,
        correlation_id: None,
        payload: vec![],
    };

    let mut stream = client
        .call_streaming::<TestStreamEnvelope>(request)
        .await
        .expect("call_streaming");

    // Verify the auto-generated correlation ID is positive.
    assert!(stream.correlation_id() > 0, "should auto-generate CID");

    let frame = stream.next().await.expect("data frame").expect("Ok");
    assert_eq!(frame.payload, vec![42]);

    let end = stream.next().await;
    assert!(end.is_none(), "stream should terminate");
}

#[rstest]
#[tokio::test]
async fn receive_streaming_works_with_pre_sent_request(correlation_id: u64) {
    let frames = vec![
        TestStreamEnvelope::data(1, correlation_id, vec![55]),
        TestStreamEnvelope::terminator(correlation_id),
    ];

    let (mut client, _server) = setup_streaming_test(frames).await;

    // Send the request manually via send_envelope.
    let request = TestStreamEnvelope::data(99, correlation_id, vec![]);
    let cid = client.send_envelope(request).await.expect("send");

    // Use receive_streaming to consume the response.
    let mut stream = client.receive_streaming::<TestStreamEnvelope>(cid);

    let frame = stream.next().await.expect("data frame").expect("Ok");
    assert_eq!(frame.payload, vec![55]);

    let end = stream.next().await;
    assert!(end.is_none(), "stream should terminate");
}
