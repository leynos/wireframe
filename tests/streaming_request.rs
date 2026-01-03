#![cfg(not(loom))]
//! Integration tests for streaming request bodies.
//!
//! These tests exercise back-pressure propagation and buffered-to-streaming
//! fallback behaviour as specified in ADR 0002.

mod common;
use std::time::Duration;

use bytes::Bytes;
use common::TestResult;
use futures::StreamExt;
use rstest::rstest;
use tokio::io::AsyncReadExt;
use wireframe::{
    extractor::StreamingBody,
    request::{RequestBodyReader, RequestBodyStream, RequestParts, body_channel},
};

#[rstest]
#[tokio::test]
async fn back_pressure_suspends_sender_when_full() -> TestResult<()> {
    let (tx, _rx) = body_channel(1);

    tx.send(Ok(Bytes::from_static(b"first"))).await?;

    // Second send should not complete immediately due to back-pressure
    let send_fut = tx.send(Ok(Bytes::from_static(b"second")));
    let timeout = tokio::time::timeout(Duration::from_millis(50), send_fut).await;

    assert!(
        timeout.is_err(),
        "send should have blocked due to back-pressure"
    );
    Ok(())
}

#[rstest]
#[tokio::test]
async fn reader_consumes_full_stream() -> TestResult<()> {
    let (tx, stream) = body_channel(4);

    tokio::spawn(async move {
        let _ = tx.send(Ok(Bytes::from_static(b"hello "))).await;
        let _ = tx.send(Ok(Bytes::from_static(b"world"))).await;
        // Sender dropped here, closing the channel
    });

    let mut reader = RequestBodyReader::new(stream);
    let mut buf = Vec::new();
    reader.read_to_end(&mut buf).await?;

    assert_eq!(buf, b"hello world");
    Ok(())
}

#[rstest]
fn buffered_fallback_uses_metadata_directly() {
    // When no streaming body is available, handlers use RequestParts directly
    let parts = RequestParts::new(42, Some(1), vec![0x01, 0x02, 0x03]);

    assert_eq!(parts.id(), 42);
    assert_eq!(parts.correlation_id(), Some(1));
    assert_eq!(parts.metadata(), &[0x01, 0x02, 0x03]);
}

#[rstest]
#[tokio::test]
async fn streaming_body_extractor_provides_reader() -> TestResult<()> {
    let (tx, stream) = body_channel(4);

    tokio::spawn(async move {
        let _ = tx.send(Ok(Bytes::from_static(b"payload"))).await;
    });

    let body = StreamingBody::new(stream);
    let mut reader = body.into_reader();

    let mut buf = Vec::new();
    reader.read_to_end(&mut buf).await?;
    assert_eq!(buf, b"payload");
    Ok(())
}

#[rstest]
#[tokio::test]
async fn streaming_body_extractor_provides_stream() -> TestResult<()> {
    let (tx, stream) = body_channel(4);

    tokio::spawn(async move {
        let _ = tx.send(Ok(Bytes::from_static(b"chunk1"))).await;
        let _ = tx.send(Ok(Bytes::from_static(b"chunk2"))).await;
    });

    let body = StreamingBody::new(stream);
    let mut stream: RequestBodyStream = body.into_stream();

    let mut chunks = Vec::new();
    while let Some(result) = stream.next().await {
        chunks.push(result?);
    }

    assert_eq!(chunks.len(), 2);
    assert_eq!(chunks.first().map(AsRef::as_ref), Some(b"chunk1".as_slice()));
    assert_eq!(chunks.get(1).map(AsRef::as_ref), Some(b"chunk2".as_slice()));
    Ok(())
}

#[rstest]
#[tokio::test]
async fn stream_error_propagates_to_handler() -> TestResult<()> {
    let (tx, stream) = body_channel(4);

    tokio::spawn(async move {
        let _ = tx.send(Ok(Bytes::from_static(b"ok"))).await;
        let _ = tx
            .send(Err(std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                "corrupt data",
            )))
            .await;
    });

    let mut stream: RequestBodyStream = stream;
    let results: Vec<_> = stream.by_ref().collect().await;

    assert_eq!(results.len(), 2);
    assert!(results.first().expect("first result").is_ok());
    assert!(results.get(1).expect("second result").is_err());
    Ok(())
}

#[rstest]
#[tokio::test]
async fn reader_surfaces_stream_error() -> TestResult<()> {
    let (tx, stream) = body_channel(4);

    tokio::spawn(async move {
        let _ = tx
            .send(Err(std::io::Error::new(
                std::io::ErrorKind::BrokenPipe,
                "connection lost",
            )))
            .await;
    });

    let mut reader = RequestBodyReader::new(stream);
    let mut buf = [0u8; 16];
    let result = reader.read(&mut buf).await;

    assert!(result.is_err());
    assert_eq!(
        result.expect_err("should be error").kind(),
        std::io::ErrorKind::BrokenPipe
    );
    Ok(())
}

#[rstest]
#[tokio::test]
async fn dropped_receiver_signals_sender() -> TestResult<()> {
    let (tx, rx) = body_channel(4);
    drop(rx);

    let result = tx.send(Ok(Bytes::from_static(b"orphaned"))).await;
    assert!(result.is_err(), "send should fail when receiver is dropped");
    Ok(())
}

#[rstest]
#[tokio::test]
async fn sequential_chunks_delivered_in_order() -> TestResult<()> {
    let (tx, stream) = body_channel(8);

    tokio::spawn(async move {
        for i in 0u8..5 {
            let chunk = Bytes::from(vec![i]);
            let _ = tx.send(Ok(chunk)).await;
        }
    });

    let chunks: Vec<_> = stream.collect().await;
    let values: Vec<u8> = chunks
        .into_iter()
        .filter_map(Result::ok)
        .flat_map(|b| b.to_vec())
        .collect();

    assert_eq!(values, vec![0, 1, 2, 3, 4]);
    Ok(())
}

#[rstest]
fn streaming_body_debug_format() {
    let (_tx, stream) = body_channel(1);
    let body = StreamingBody::new(stream);
    let debug_str = format!("{body:?}");
    assert!(debug_str.contains("StreamingBody"));
}
