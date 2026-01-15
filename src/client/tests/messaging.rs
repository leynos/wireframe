//! Unit tests for client messaging APIs with correlation ID support.

use std::sync::{
    Arc,
    atomic::{AtomicUsize, Ordering},
};

use bytes::Bytes;
use futures::{SinkExt, StreamExt};
use rstest::rstest;
use tokio::net::TcpListener;
use tokio_util::codec::{Framed, LengthDelimitedCodec};
use wireframe_testing::{ServerMode, process_frame};

use crate::{
    WireframeClient,
    app::{Envelope, Packet},
    client::ClientError,
};

/// Spawn a test server with the specified mode.
async fn spawn_test_server(
    mode: ServerMode,
) -> (std::net::SocketAddr, tokio::task::JoinHandle<()>) {
    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind listener");
    let addr = listener.local_addr().expect("listener addr");

    let handle = tokio::spawn(async move {
        let (stream, _) = listener.accept().await.expect("accept client");
        let mut framed = Framed::new(stream, LengthDelimitedCodec::new());

        while let Some(Ok(bytes)) = framed.next().await {
            let Some(response_bytes) = process_frame(mode, &bytes) else {
                break;
            };
            if framed.send(Bytes::from(response_bytes)).await.is_err() {
                break;
            }
        }
    });

    (addr, handle)
}

/// Spawn a TCP listener and return an echo server that preserves correlation IDs.
async fn spawn_envelope_echo_server() -> (std::net::SocketAddr, tokio::task::JoinHandle<()>) {
    spawn_test_server(ServerMode::Echo).await
}

/// Spawn a server that returns envelopes with a different correlation ID.
async fn spawn_mismatched_correlation_server() -> (std::net::SocketAddr, tokio::task::JoinHandle<()>)
{
    spawn_test_server(ServerMode::Mismatch).await
}

#[tokio::test]
async fn next_correlation_id_generates_sequential_unique_ids() {
    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind listener");
    let addr = listener.local_addr().expect("listener addr");

    let accept = tokio::spawn(async move {
        let (stream, _) = listener.accept().await.expect("accept");
        stream
    });

    let client = WireframeClient::builder()
        .connect(addr)
        .await
        .expect("connect client");

    let _server = accept.await.expect("join accept");

    // Generate several correlation IDs and verify they are sequential and unique.
    let id1 = client.next_correlation_id();
    let id2 = client.next_correlation_id();
    let id3 = client.next_correlation_id();

    assert_eq!(id1, 1, "first correlation ID should be 1");
    assert_eq!(id2, 2, "second correlation ID should be 2");
    assert_eq!(id3, 3, "third correlation ID should be 3");
}

#[tokio::test]
async fn send_envelope_auto_generates_correlation_id_when_none() {
    let (addr, server) = spawn_envelope_echo_server().await;

    let mut client = WireframeClient::builder()
        .connect(addr)
        .await
        .expect("connect client");

    // Create envelope without correlation ID.
    let envelope = Envelope::new(42, None, vec![1, 2, 3]);
    assert!(envelope.correlation_id().is_none());

    // send_envelope should auto-generate a correlation ID.
    let correlation_id = client.send_envelope(envelope).await.expect("send envelope");
    assert!(
        correlation_id > 0,
        "auto-generated correlation ID should be positive"
    );

    // Clean up.
    server.abort();
}

#[tokio::test]
async fn send_envelope_preserves_existing_correlation_id() {
    let (addr, server) = spawn_envelope_echo_server().await;

    let mut client = WireframeClient::builder()
        .connect(addr)
        .await
        .expect("connect client");

    // Create envelope with explicit correlation ID.
    let explicit_id = 12345u64;
    let envelope = Envelope::new(42, Some(explicit_id), vec![1, 2, 3]);

    let correlation_id = client.send_envelope(envelope).await.expect("send envelope");
    assert_eq!(
        correlation_id, explicit_id,
        "should preserve explicit correlation ID"
    );

    server.abort();
}

#[tokio::test]
async fn receive_envelope_returns_envelope_with_correlation_id() {
    let (addr, server) = spawn_envelope_echo_server().await;

    let mut client = WireframeClient::builder()
        .connect(addr)
        .await
        .expect("connect client");

    // Send an envelope.
    let request = Envelope::new(42, None, vec![1, 2, 3]);
    let sent_correlation_id = client.send_envelope(request).await.expect("send envelope");

    // Receive the echoed envelope.
    let response: Envelope = client.receive_envelope().await.expect("receive envelope");

    assert_eq!(
        response.correlation_id(),
        Some(sent_correlation_id),
        "response should have the same correlation ID"
    );
    assert_eq!(response.id(), 42);
    assert_eq!(response.into_parts().payload(), &[1, 2, 3]);

    server.abort();
}

#[tokio::test]
async fn call_correlated_validates_matching_correlation_id() {
    let (addr, server) = spawn_envelope_echo_server().await;

    let mut client = WireframeClient::builder()
        .connect(addr)
        .await
        .expect("connect client");

    let request = Envelope::new(42, None, vec![1, 2, 3]);
    let response: Envelope = client
        .call_correlated(request)
        .await
        .expect("call_correlated should succeed");

    assert!(
        response.correlation_id().is_some(),
        "response should have correlation ID"
    );

    server.abort();
}

#[tokio::test]
async fn call_correlated_returns_error_on_correlation_mismatch() {
    let (addr, server) = spawn_mismatched_correlation_server().await;

    let mut client = WireframeClient::builder()
        .connect(addr)
        .await
        .expect("connect client");

    let request = Envelope::new(42, None, vec![1, 2, 3]);
    let result: Result<Envelope, ClientError> = client.call_correlated(request).await;

    match result {
        Err(ClientError::CorrelationMismatch { expected, received }) => {
            assert!(expected.is_some(), "expected correlation ID should be set");
            assert!(received.is_some(), "received correlation ID should be set");
            assert_ne!(
                expected, received,
                "mismatched correlation IDs should differ"
            );
        }
        Ok(_) => panic!("expected CorrelationMismatch error, got Ok"),
        Err(e) => panic!("expected CorrelationMismatch error, got {e:?}"),
    }

    server.abort();
}

#[tokio::test]
async fn call_correlated_invokes_error_hook_on_mismatch() {
    let (addr, server) = spawn_mismatched_correlation_server().await;

    let error_count = Arc::new(AtomicUsize::new(0));
    let count = error_count.clone();

    let mut client = WireframeClient::builder()
        .on_error(move |_err| {
            let count = count.clone();
            async move {
                count.fetch_add(1, Ordering::SeqCst);
            }
        })
        .connect(addr)
        .await
        .expect("connect client");

    let request = Envelope::new(42, None, vec![1, 2, 3]);
    let _result: Result<Envelope, ClientError> = client.call_correlated(request).await;

    assert_eq!(
        error_count.load(Ordering::SeqCst),
        1,
        "error hook should be invoked on correlation mismatch"
    );

    server.abort();
}

#[tokio::test]
async fn multiple_sequential_calls_get_unique_correlation_ids() {
    let (addr, server) = spawn_envelope_echo_server().await;

    let mut client = WireframeClient::builder()
        .connect(addr)
        .await
        .expect("connect client");

    let mut correlation_ids = Vec::new();

    for i in 0u8..5 {
        let request = Envelope::new(u32::from(i), None, vec![i]);
        let response: Envelope = client
            .call_correlated(request)
            .await
            .expect("call should succeed");
        correlation_ids.push(
            response
                .correlation_id()
                .expect("should have correlation ID"),
        );
    }

    // All correlation IDs should be unique.
    let mut sorted = correlation_ids.clone();
    sorted.sort_unstable();
    sorted.dedup();
    assert_eq!(
        sorted.len(),
        correlation_ids.len(),
        "all correlation IDs should be unique"
    );

    server.abort();
}

#[rstest]
#[case(vec![])]
#[case(vec![0u8; 100])]
#[case(vec![255u8; 500])]
#[tokio::test]
async fn round_trip_with_various_payload_sizes(#[case] payload: Vec<u8>) {
    let (addr, server) = spawn_envelope_echo_server().await;

    let mut client = WireframeClient::builder()
        .connect(addr)
        .await
        .expect("connect client");

    let request = Envelope::new(1, None, payload.clone());
    let response: Envelope = client
        .call_correlated(request)
        .await
        .expect("call should succeed");

    assert_eq!(response.into_parts().payload(), payload.as_slice());

    server.abort();
}

#[tokio::test]
async fn receive_envelope_returns_disconnected_on_closed_connection() {
    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind listener");
    let addr = listener.local_addr().expect("listener addr");

    let accept = tokio::spawn(async move {
        let (stream, _) = listener.accept().await.expect("accept");
        // Immediately drop the connection.
        drop(stream);
    });

    let mut client = WireframeClient::builder()
        .connect(addr)
        .await
        .expect("connect client");

    accept.await.expect("join accept");

    let result: Result<Envelope, ClientError> = client.receive_envelope().await;
    assert!(
        matches!(result, Err(ClientError::Disconnected)),
        "expected Disconnected error"
    );
}
