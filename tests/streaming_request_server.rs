//! End-to-end TCP lifecycle coverage for streaming request bodies.
//!
//! These tests exercise the streaming dispatch slot over a real socket: a body
//! split across two frames reaches the handler, and a connection that ends
//! mid-body reports `UnexpectedEof` to that handler while the server still
//! shuts the connection down cleanly.
//!
//! The client frames with [`LengthDelimitedCodec`] rather than hand-built
//! length prefixes, so the tests speak exactly the wire format the runtime
//! decodes.
//!
//! # Examples
//!
//! ```ignore
//! let (address, server) = serve_once(collecting_handler(Arc::clone(&received))).await?;
//! let mut client = connect(address).await?;
//! send_request(&mut client, &first_frame).await?;
//! // ... drive the exchange, then prove the server finished cleanly:
//! finish_server(server).await?;
//! ```

#![forbid(unsafe_code)]

use std::{
    io,
    net::SocketAddr,
    sync::{Arc, Mutex},
};

use bytes::Bytes;
use futures::{SinkExt, StreamExt};
use tokio::{
    net::{TcpListener, TcpStream},
    task::JoinHandle,
};
use tokio_util::codec::{Framed, LengthDelimitedCodec};
use wireframe::{
    app::{Envelope, StreamingHandler, WireframeApp},
    request::{RequestBodyStream, RequestParts},
    serializer::{BincodeSerializer, Serializer},
    test_helpers,
};
use wireframe_testing::TestResult;

/// Route identifier served by the streaming handler in these tests.
const STREAM_ROUTE: u32 = 90;
/// Per-connection frame budget.
const BUFFER_CAPACITY: usize = 512;

/// Client side of one connection, framed with the runtime's length prefix.
type Client = Framed<TcpStream, LengthDelimitedCodec>;

/// Serve exactly one connection with the supplied streaming handler.
///
/// The server task is returned rather than detached, so a test can prove the
/// connection handler finished without an error instead of abandoning it.
#[expect(
    deprecated,
    reason = "end-to-end coverage of the legacy builder connection API"
)]
async fn serve_once(
    handler: StreamingHandler,
) -> TestResult<(SocketAddr, JoinHandle<io::Result<()>>)> {
    let app: WireframeApp = WireframeApp::new()?
        .buffer_capacity(BUFFER_CAPACITY)
        .with_message_assembler(test_helpers::TestAssembler)
        .streaming_route(STREAM_ROUTE, handler)?;
    let listener = TcpListener::bind("127.0.0.1:0").await?;
    let address = listener.local_addr()?;
    let server = tokio::spawn(async move {
        let (stream, _) = listener.accept().await?;
        app.handle_connection_result(stream).await
    });
    Ok((address, server))
}

/// Await the server task, reporting a join failure or a connection error.
async fn finish_server(server: JoinHandle<io::Result<()>>) -> TestResult {
    match server.await {
        Ok(result) => Ok(result?),
        Err(error) => Err(format!("the server task failed: {error}").into()),
    }
}

/// Connect a length-delimited client to `address`.
async fn connect(address: SocketAddr) -> TestResult<Client> {
    let stream = TcpStream::connect(address).await?;
    Ok(Framed::new(stream, LengthDelimitedCodec::new()))
}

/// Send one request frame carrying `payload` to the streaming route.
async fn send_request(client: &mut Client, payload: &[u8]) -> TestResult {
    let envelope = Envelope::new(STREAM_ROUTE, Some(1), payload.to_vec());
    let encoded = BincodeSerializer.serialize(&envelope)?;
    client.send(Bytes::from(encoded)).await?;
    Ok(())
}

/// Close the client's sending half.
///
/// The codec encodes both `Bytes` and `BytesMut`, so the sink item type is
/// named explicitly rather than inferred.
async fn close_client(client: &mut Client) -> TestResult {
    SinkExt::<Bytes>::close(client).await?;
    Ok(())
}

/// Read one reply frame and return its payload.
async fn read_reply(client: &mut Client) -> TestResult<Vec<u8>> {
    let Some(frame) = client.next().await else {
        return Err("the reply frame is missing".into());
    };
    let (envelope, _): (Envelope, usize) = BincodeSerializer.deserialize(&frame?)?;
    Ok(envelope.payload_bytes().to_vec())
}

/// Handler that collects the request body and answers once the body ends.
fn collecting_handler(received: Arc<Mutex<Vec<u8>>>) -> StreamingHandler {
    Arc::new(move |_parts: RequestParts, body: RequestBodyStream| {
        Box::pin(collect_body(body, Arc::clone(&received)))
    })
}

/// Read `body` to its end, appending each chunk to `received`.
async fn collect_body(
    mut body: RequestBodyStream,
    received: Arc<Mutex<Vec<u8>>>,
) -> Option<Vec<u8>> {
    while let Some(chunk) = body.next().await {
        let bytes = chunk.ok()?;
        if let Ok(mut buffer) = received.lock() {
            buffer.extend_from_slice(&bytes);
        }
    }
    Some(b"done".to_vec())
}

/// Handler that records how the request body ended and never replies.
fn observing_handler(observed: Arc<Mutex<Option<io::ErrorKind>>>) -> StreamingHandler {
    Arc::new(move |_parts: RequestParts, body: RequestBodyStream| {
        Box::pin(observe_body_end(body, Arc::clone(&observed)))
    })
}

/// Record the body's terminal error, if any, and never produce a reply.
async fn observe_body_end(
    mut body: RequestBodyStream,
    observed: Arc<Mutex<Option<io::ErrorKind>>>,
) -> Option<Vec<u8>> {
    while let Some(item) = body.next().await {
        if let Err(error) = item {
            record_kind(&observed, error.kind());
            return None;
        }
    }
    None
}

/// Store the observed error kind, ignoring a poisoned lock.
fn record_kind(observed: &Mutex<Option<io::ErrorKind>>, kind: io::ErrorKind) {
    if let Ok(mut slot) = observed.lock() {
        *slot = Some(kind);
    }
}

/// Assert the streaming reply reached the client.
fn assert_reply(reply: &[u8]) {
    assert_eq!(
        reply, b"done",
        "the streaming reply should reach the client"
    );
}

/// Assert the handler saw the body in frame order.
fn assert_received(received: &[u8]) {
    assert_eq!(
        received, b"alphabeta",
        "the handler should receive the body in frame order"
    );
}

/// Assert the handler observed the truncated body.
fn assert_truncated(kind: Option<io::ErrorKind>) {
    assert_eq!(
        kind,
        Some(io::ErrorKind::UnexpectedEof),
        "the handler should observe the truncated body"
    );
}

/// Assert the server sent no reply frame.
fn assert_no_reply(has_reply: bool) {
    assert!(!has_reply, "an errored body sends no streaming reply");
}

/// A body delivered across two frames reaches the handler over TCP.
#[tokio::test]
async fn streaming_body_reaches_the_handler_over_tcp() -> TestResult {
    let received = Arc::new(Mutex::new(Vec::new()));
    let (address, server) = serve_once(collecting_handler(Arc::clone(&received))).await?;
    let mut client = connect(address).await?;

    let first = test_helpers::first_frame_payload(1.into(), b"alpha", false, None)?;
    let last = test_helpers::continuation_frame_payload(1.into(), 1.into(), b"beta", true)?;
    send_request(&mut client, &first).await?;
    send_request(&mut client, &last).await?;

    let reply = read_reply(&mut client).await?;
    close_client(&mut client).await?;
    finish_server(server).await?;
    let body = received
        .lock()
        .map_err(|_| "handler buffer poisoned")?
        .clone();

    assert_reply(&reply);
    assert_received(&body);
    Ok(())
}

/// A connection closed mid-body reports `UnexpectedEof` and closes cleanly.
#[tokio::test]
async fn closing_the_connection_mid_body_reports_unexpected_eof() -> TestResult {
    let observed: Arc<Mutex<Option<io::ErrorKind>>> = Arc::new(Mutex::new(None));
    let (address, server) = serve_once(observing_handler(Arc::clone(&observed))).await?;
    let mut client = connect(address).await?;

    let first = test_helpers::first_frame_payload(2.into(), b"partial", false, None)?;
    send_request(&mut client, &first).await?;
    close_client(&mut client).await?;
    let has_reply = client.next().await.is_some();
    finish_server(server).await?;
    let kind = *observed.lock().map_err(|_| "observation slot poisoned")?;

    assert_truncated(kind);
    assert_no_reply(has_reply);
    Ok(())
}
