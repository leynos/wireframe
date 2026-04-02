//! Tests for streaming request body types.
//!
//! These tests verify stream consumption, back-pressure behaviour, error
//! propagation, and the `AsyncRead` adapter.
use std::io;

use bytes::Bytes;
use futures::StreamExt;
use tokio::io::AsyncReadExt;

use super::{RequestBodyReader, RequestBodyStream, body_channel};

#[tokio::test]
async fn request_body_stream_yields_chunks() {
    let chunks = vec![
        Ok(Bytes::from_static(b"hello")),
        Ok(Bytes::from_static(b" world")),
    ];
    let stream: RequestBodyStream = Box::pin(futures::stream::iter(chunks));

    let collected: Vec<_> = stream.collect().await;
    assert_eq!(collected.len(), 2);
    assert!(collected.iter().all(Result::is_ok));
}

#[tokio::test]
async fn async_read_adapter_reads_stream() {
    let chunks = vec![Ok(Bytes::from_static(b"test"))];
    let stream: RequestBodyStream = Box::pin(futures::stream::iter(chunks));
    let mut reader = RequestBodyReader::new(stream);

    let mut buf = [0u8; 4];
    let n = reader.read(&mut buf).await.expect("read should succeed");
    assert_eq!(n, 4);
    assert_eq!(&buf, b"test");
}

#[tokio::test]
async fn reader_consumes_multiple_chunks() {
    let chunks = vec![
        Ok(Bytes::from_static(b"hello ")),
        Ok(Bytes::from_static(b"world")),
    ];
    let stream: RequestBodyStream = Box::pin(futures::stream::iter(chunks));
    let mut reader = RequestBodyReader::new(stream);

    let mut buf = Vec::new();
    reader
        .read_to_end(&mut buf)
        .await
        .expect("read_to_end should succeed");
    assert_eq!(buf, b"hello world");
}

#[tokio::test]
async fn stream_propagates_io_error() {
    let chunks = vec![
        Ok(Bytes::from_static(b"ok")),
        Err(io::Error::new(io::ErrorKind::InvalidData, "corrupt")),
    ];
    let stream: RequestBodyStream = Box::pin(futures::stream::iter(chunks));

    let results: Vec<_> = stream.collect().await;
    assert!(
        matches!(results.as_slice(), [Ok(_), Err(error)] if error.kind() == io::ErrorKind::InvalidData)
    );
}

#[tokio::test]
async fn reader_surfaces_stream_error() {
    let chunks = vec![Err(io::Error::new(
        io::ErrorKind::BrokenPipe,
        "disconnected",
    ))];
    let stream: RequestBodyStream = Box::pin(futures::stream::iter(chunks));
    let mut reader = RequestBodyReader::new(stream);

    let mut buf = [0u8; 16];
    let err = reader
        .read(&mut buf)
        .await
        .expect_err("read should fail with BrokenPipe");
    assert_eq!(err.kind(), io::ErrorKind::BrokenPipe);
}

#[tokio::test]
async fn body_channel_delivers_chunks() {
    let (tx, stream) = body_channel(4);

    tx.send(Ok(Bytes::from_static(b"chunk1")))
        .await
        .expect("send should succeed");
    tx.send(Ok(Bytes::from_static(b"chunk2")))
        .await
        .expect("send should succeed");
    drop(tx);

    let chunks: Vec<_> = stream.collect().await;
    assert_eq!(chunks.len(), 2);
}

#[tokio::test]
async fn body_channel_back_pressure() {
    let (tx, _rx) = body_channel(1);

    tx.send(Ok(Bytes::from_static(b"first")))
        .await
        .expect("first send should succeed");

    // Channel is full; try_send should fail
    let result = tx.try_send(Ok(Bytes::from_static(b"second")));
    assert!(result.is_err(), "try_send should fail when channel is full");
}

#[tokio::test]
async fn body_channel_sender_detects_dropped_receiver() {
    let (tx, rx) = body_channel(1);
    drop(rx);

    let result = tx.send(Ok(Bytes::from_static(b"orphan"))).await;
    assert!(result.is_err(), "send should fail when receiver is dropped");
}

#[test]
fn reader_into_inner_returns_stream() {
    let chunks = vec![Ok(Bytes::from_static(b"test"))];
    let stream: RequestBodyStream = Box::pin(futures::stream::iter(chunks));
    let reader = RequestBodyReader::new(stream);

    let _recovered: RequestBodyStream = reader.into_inner();
    // Compiles and type-checks; stream is returned
}
