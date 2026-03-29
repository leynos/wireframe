//! Tests for streaming request body types.
//!
//! These tests verify stream consumption, back-pressure behaviour, error
//! propagation, and the `AsyncRead` adapter.
use std::{future::Future, io};

use bytes::Bytes;
use futures::StreamExt;
use tokio::io::AsyncReadExt;

use super::{RequestBodyReader, RequestBodyStream, body_channel};

fn run_async_test<T>(future: impl Future<Output = T>) -> T {
    let runtime = match tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
    {
        Ok(runtime) => runtime,
        Err(error) => panic!("failed to build tokio runtime for request tests: {error}"),
    };
    runtime.block_on(future)
}

#[test]
fn request_body_stream_yields_chunks() {
    run_async_test(async {
        let chunks = vec![
            Ok(Bytes::from_static(b"hello")),
            Ok(Bytes::from_static(b" world")),
        ];
        let stream: RequestBodyStream = Box::pin(futures::stream::iter(chunks));

        let collected: Vec<_> = stream.collect().await;
        assert_eq!(collected.len(), 2);
        assert!(collected.iter().all(Result::is_ok));
    });
}

#[test]
fn async_read_adapter_reads_stream() {
    run_async_test(async {
        let chunks = vec![Ok(Bytes::from_static(b"test"))];
        let stream: RequestBodyStream = Box::pin(futures::stream::iter(chunks));
        let mut reader = RequestBodyReader::new(stream);

        let mut buf = [0u8; 4];
        let n = match reader.read(&mut buf).await {
            Ok(bytes_read) => bytes_read,
            Err(error) => panic!("read should succeed: {error}"),
        };
        assert_eq!(n, 4);
        assert_eq!(&buf, b"test");
    });
}

#[test]
fn reader_consumes_multiple_chunks() {
    run_async_test(async {
        let chunks = vec![
            Ok(Bytes::from_static(b"hello ")),
            Ok(Bytes::from_static(b"world")),
        ];
        let stream: RequestBodyStream = Box::pin(futures::stream::iter(chunks));
        let mut reader = RequestBodyReader::new(stream);

        let mut buf = Vec::new();
        if let Err(error) = reader.read_to_end(&mut buf).await {
            panic!("read_to_end should succeed: {error}");
        }
        assert_eq!(buf, b"hello world");
    });
}

#[test]
fn stream_propagates_io_error() {
    run_async_test(async {
        let chunks = vec![
            Ok(Bytes::from_static(b"ok")),
            Err(io::Error::new(io::ErrorKind::InvalidData, "corrupt")),
        ];
        let stream: RequestBodyStream = Box::pin(futures::stream::iter(chunks));

        let results: Vec<_> = stream.collect().await;
        assert!(matches!(results.as_slice(), [Ok(_), Err(_)]));
    });
}

#[test]
fn reader_surfaces_stream_error() {
    run_async_test(async {
        let chunks = vec![Err(io::Error::new(
            io::ErrorKind::BrokenPipe,
            "disconnected",
        ))];
        let stream: RequestBodyStream = Box::pin(futures::stream::iter(chunks));
        let mut reader = RequestBodyReader::new(stream);

        let mut buf = [0u8; 16];
        match reader.read(&mut buf).await {
            Ok(bytes_read) => panic!("read should fail, got {bytes_read} bytes"),
            Err(err) => assert_eq!(err.kind(), io::ErrorKind::BrokenPipe),
        }
    });
}

#[test]
fn body_channel_delivers_chunks() {
    run_async_test(async {
        let (tx, stream) = body_channel(4);

        if let Err(error) = tx.send(Ok(Bytes::from_static(b"chunk1"))).await {
            panic!("send should succeed: {error}");
        }
        if let Err(error) = tx.send(Ok(Bytes::from_static(b"chunk2"))).await {
            panic!("send should succeed: {error}");
        }
        drop(tx);

        let chunks: Vec<_> = stream.collect().await;
        assert_eq!(chunks.len(), 2);
    });
}

#[test]
fn body_channel_back_pressure() {
    run_async_test(async {
        let (tx, _rx) = body_channel(1);

        if let Err(error) = tx.send(Ok(Bytes::from_static(b"first"))).await {
            panic!("first send should succeed: {error}");
        }

        // Channel is full; try_send should fail
        let result = tx.try_send(Ok(Bytes::from_static(b"second")));
        assert!(result.is_err(), "try_send should fail when channel is full");
    });
}

#[test]
fn body_channel_sender_detects_dropped_receiver() {
    run_async_test(async {
        let (tx, rx) = body_channel(1);
        drop(rx);

        let result = tx.send(Ok(Bytes::from_static(b"orphan"))).await;
        assert!(result.is_err(), "send should fail when receiver is dropped");
    });
}

#[test]
fn reader_into_inner_returns_stream() {
    let chunks = vec![Ok(Bytes::from_static(b"test"))];
    let stream: RequestBodyStream = Box::pin(futures::stream::iter(chunks));
    let reader = RequestBodyReader::new(stream);

    let _recovered: RequestBodyStream = reader.into_inner();
    // Compiles and type-checks; stream is returned
}
