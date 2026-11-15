//! End-to-end TCP lifecycle coverage for streaming request bodies.
//!
//! These tests exercise the streaming dispatch slot over a real socket: a body
//! split across two frames reaches the handler, and a connection that ends
//! mid-body reports `UnexpectedEof` to that handler while the server still
//! shuts the connection down cleanly.

#![forbid(unsafe_code)]

use std::{
    io,
    sync::{Arc, Mutex},
};

use futures::StreamExt;
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::{TcpListener, TcpStream},
};
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

/// Serve exactly one connection with the supplied streaming handler.
#[expect(
    deprecated,
    reason = "end-to-end coverage of the legacy builder connection API"
)]
async fn serve_once(handler: StreamingHandler) -> TestResult<std::net::SocketAddr> {
    let app: WireframeApp = WireframeApp::new()?
        .buffer_capacity(BUFFER_CAPACITY)
        .with_message_assembler(test_helpers::TestAssembler)
        .streaming_route(STREAM_ROUTE, handler)?;
    let listener = TcpListener::bind("127.0.0.1:0").await?;
    let address = listener.local_addr()?;
    tokio::spawn(async move {
        let (stream, _) = listener.accept().await?;
        app.handle_connection_result(stream).await
    });
    Ok(address)
}

/// Build a length-delimited request frame carrying `payload`.
fn request_frame(payload: Vec<u8>) -> TestResult<Vec<u8>> {
    let envelope = Envelope::new(STREAM_ROUTE, Some(1), payload);
    let encoded = BincodeSerializer.serialize(&envelope)?;
    let len = u32::try_from(encoded.len())?;
    let mut framed = Vec::with_capacity(encoded.len() + 4);
    framed.extend_from_slice(&len.to_be_bytes());
    framed.extend_from_slice(&encoded);
    Ok(framed)
}

/// Read one length-delimited reply and return its payload.
async fn read_reply(stream: &mut TcpStream) -> TestResult<Vec<u8>> {
    let mut len = [0u8; 4];
    stream.read_exact(&mut len).await?;
    let mut frame = vec![0u8; u32::from_be_bytes(len) as usize];
    stream.read_exact(&mut frame).await?;
    let (envelope, _): (Envelope, usize) = BincodeSerializer.deserialize(&frame)?;
    Ok(envelope.payload_bytes().to_vec())
}

/// A body delivered across two frames reaches the handler over TCP.
#[tokio::test]
async fn streaming_body_reaches_the_handler_over_tcp() -> TestResult<()> {
    let received = Arc::new(Mutex::new(Vec::new()));
    let handler: StreamingHandler = {
        let received = Arc::clone(&received);
        Arc::new(move |_parts: RequestParts, mut body: RequestBodyStream| {
            let received = Arc::clone(&received);
            Box::pin(async move {
                while let Some(chunk) = body.next().await {
                    let Some(chunk) = chunk.ok() else {
                        return None;
                    };
                    if let Ok(mut buffer) = received.lock() {
                        buffer.extend_from_slice(&chunk);
                    }
                }
                Some(b"done".to_vec())
            })
        })
    };
    let address = serve_once(handler).await?;

    let first = test_helpers::first_frame_payload(1.into(), b"alpha", false, None)?;
    let last = test_helpers::continuation_frame_payload(1.into(), 1.into(), b"beta", true)?;
    let mut client = TcpStream::connect(address).await?;
    client.write_all(&request_frame(first)?).await?;
    client.write_all(&request_frame(last)?).await?;
    client.flush().await?;

    let reply = read_reply(&mut client).await?;
    client.shutdown().await?;

    assert_eq!(
        reply, b"done",
        "the streaming reply should reach the client"
    );
    let received = received
        .lock()
        .map_err(|_| "handler buffer poisoned")?
        .clone();
    assert_eq!(
        received, b"alphabeta",
        "the handler should receive the body in frame order"
    );
    Ok(())
}

/// A connection closed mid-body reports `UnexpectedEof` and closes cleanly.
#[tokio::test]
async fn closing_the_connection_mid_body_reports_unexpected_eof() -> TestResult<()> {
    let observed: Arc<Mutex<Option<io::ErrorKind>>> = Arc::new(Mutex::new(None));
    let handler: StreamingHandler = {
        let observed = Arc::clone(&observed);
        Arc::new(move |_parts: RequestParts, mut body: RequestBodyStream| {
            let observed = Arc::clone(&observed);
            Box::pin(async move {
                while let Some(item) = body.next().await {
                    if let Err(error) = item {
                        if let Ok(mut slot) = observed.lock() {
                            *slot = Some(error.kind());
                        }
                        return None;
                    }
                }
                None
            })
        })
    };
    let address = serve_once(handler).await?;

    let first = test_helpers::first_frame_payload(2.into(), b"partial", false, None)?;
    let mut client = TcpStream::connect(address).await?;
    client.write_all(&request_frame(first)?).await?;
    client.flush().await?;
    client.shutdown().await?;

    let mut trailing = Vec::new();
    client.read_to_end(&mut trailing).await?;

    let kind = *observed.lock().map_err(|_| "observation slot poisoned")?;
    assert_eq!(
        kind,
        Some(io::ErrorKind::UnexpectedEof),
        "the handler should observe the truncated body"
    );
    assert!(
        trailing.is_empty(),
        "an errored body sends no streaming reply"
    );
    Ok(())
}
