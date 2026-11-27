//! Unit tests for client messaging APIs with correlation ID support.

use std::{
    future::Future,
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    },
};

use rstest::rstest;
use tokio::net::TcpStream;
use wireframe_testing::ServerMode;

use super::helpers::{FrameServerHandle, TestResult, spawn_frame_server, spawn_listener};
use crate::{
    WireframeError,
    app::{Envelope, Packet},
    client::{ClientError, WireframeClient},
    correlation::CorrelatableFrame,
    rewind_stream::RewindStream,
    serializer::BincodeSerializer,
    test_helpers::ServerTaskGuard,
};

/// Error type used by the guarded bodies in these tests.
type BodyError = Box<dyn std::error::Error + Send + Sync>;

/// Client returned by [`connect_with_guarded_accept`].
type AcceptClient = WireframeClient<BincodeSerializer, RewindStream<TcpStream>>;

/// Bind a single-accept listener and connect to it with the accept task owned.
///
/// The accept task is guarded before the connection is attempted, so a failed
/// connect aborts and awaits it rather than leaving it detached. On success the
/// accepted stream is returned too, so the caller decides when the peer sees
/// the connection close.
async fn connect_with_guarded_accept() -> TestResult<(AcceptClient, TcpStream)> {
    let (addr, accept) = spawn_listener().await?;

    let (client, accepted) = ServerTaskGuard::new(accept)
        .run_and_join(async move {
            let client = WireframeClient::builder().connect(addr).await?;
            Ok::<_, BodyError>(client)
        })
        .await?;

    Ok((client, accepted?))
}

/// Spawn a TCP listener and return an echo server that preserves correlation IDs.
async fn spawn_envelope_echo_server() -> TestResult<(std::net::SocketAddr, FrameServerHandle)> {
    spawn_frame_server(ServerMode::Echo).await
}

/// Spawn a server that returns envelopes with a different correlation ID.
async fn spawn_mismatched_correlation_server()
-> TestResult<(std::net::SocketAddr, FrameServerHandle)> {
    spawn_frame_server(ServerMode::Mismatch).await
}

/// Run `body` against a spawned server, then reap the server.
///
/// The body owns the client and keeps its assertions inside the guarded region,
/// so a failed connection or a failed assertion still stops and awaits the
/// server instead of leaving it running. A body panic is resumed once cleanup
/// is complete, and the server's own `io::Result` is applied on success.
async fn run_guarded<T, Fut>(server: FrameServerHandle, body: Fut) -> TestResult<T>
where
    Fut: Future<Output = Result<T, BodyError>>,
{
    let (value, server_result) = ServerTaskGuard::new(server).run_and_join(body).await?;
    server_result?;
    Ok(value)
}

/// Record one error-hook invocation.
///
/// Returns a ready future rather than borrowing the counter, so the hook's
/// future stays `'static` without taking ownership of the `Arc`.
fn count_error(count: &Arc<AtomicUsize>) -> std::future::Ready<()> {
    count.fetch_add(1, Ordering::SeqCst);
    std::future::ready(())
}

#[tokio::test]
async fn next_correlation_id_generates_sequential_unique_ids() {
    let (client, _server_stream) = connect_with_guarded_accept()
        .await
        .expect("connect under a guarded accept");

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
    let (addr, server) = spawn_envelope_echo_server()
        .await
        .expect("spawn echo server");

    run_guarded(server, async move {
        let mut client = WireframeClient::builder().connect(addr).await?;

        // Create envelope without correlation ID.
        let envelope = Envelope::new(42, None, vec![1, 2, 3]);
        assert!(
            envelope.correlation_id().is_none(),
            "assertion failed: envelope.correlation_id().is_none()"
        );

        // send_envelope should auto-generate a correlation ID.
        let correlation_id = client.send_envelope(envelope).await?;
        assert!(
            correlation_id > 0,
            "auto-generated correlation ID should be positive"
        );

        drop(client);
        Ok::<(), BodyError>(())
    })
    .await
    .expect("send envelope with auto-generated correlation ID");
}

#[tokio::test]
async fn send_envelope_preserves_existing_correlation_id() {
    let (addr, server) = spawn_envelope_echo_server()
        .await
        .expect("spawn echo server");

    run_guarded(server, async move {
        let mut client = WireframeClient::builder().connect(addr).await?;

        // Create envelope with explicit correlation ID.
        let explicit_id = 12345u64;
        let envelope = Envelope::new(42, Some(explicit_id), vec![1, 2, 3]);

        let correlation_id = client.send_envelope(envelope).await?;
        assert_eq!(
            correlation_id, explicit_id,
            "should preserve explicit correlation ID"
        );

        drop(client);
        Ok::<(), BodyError>(())
    })
    .await
    .expect("send envelope with explicit correlation ID");
}

#[tokio::test]
async fn receive_envelope_returns_envelope_with_correlation_id() {
    let (addr, server) = spawn_envelope_echo_server()
        .await
        .expect("spawn echo server");

    run_guarded(server, async move {
        let mut client = WireframeClient::builder().connect(addr).await?;

        // Send an envelope.
        let request = Envelope::new(42, None, vec![1, 2, 3]);
        let sent_correlation_id = client.send_envelope(request).await?;

        // Receive the echoed envelope.
        let response: Envelope = client.receive_envelope().await?;

        assert_eq!(
            response.correlation_id(),
            Some(sent_correlation_id),
            "response should have the same correlation ID"
        );
        assert_eq!(response.id(), 42, "response.id() should equal 42");
        assert_eq!(
            response.into_parts().into_payload(),
            &[1, 2, 3],
            "response.into_parts().into_payload() should equal &[1, 2, 3]"
        );

        drop(client);
        Ok::<(), BodyError>(())
    })
    .await
    .expect("receive echoed envelope");
}

#[tokio::test]
async fn call_correlated_validates_matching_correlation_id() {
    let (addr, server) = spawn_envelope_echo_server()
        .await
        .expect("spawn echo server");

    run_guarded(server, async move {
        let mut client = WireframeClient::builder().connect(addr).await?;

        let request = Envelope::new(42, None, vec![1, 2, 3]);
        let response: Envelope = client
            .call_correlated(request)
            .await
            .expect("call_correlated should succeed");

        assert!(
            response.correlation_id().is_some(),
            "response should have correlation ID"
        );

        drop(client);
        Ok::<(), BodyError>(())
    })
    .await
    .expect("call_correlated with a matching correlation ID");
}

#[tokio::test]
async fn call_correlated_returns_error_on_correlation_mismatch() {
    let (addr, server) = spawn_mismatched_correlation_server()
        .await
        .expect("spawn mismatched server");

    run_guarded(server, async move {
        let mut client = WireframeClient::builder().connect(addr).await?;

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

        drop(client);
        Ok::<(), BodyError>(())
    })
    .await
    .expect("call_correlated with a mismatched correlation ID");
}

#[tokio::test]
async fn call_correlated_invokes_error_hook_on_mismatch() {
    let (addr, server) = spawn_mismatched_correlation_server()
        .await
        .expect("spawn mismatched server");

    run_guarded(server, async move {
        let error_count = Arc::new(AtomicUsize::new(0));
        let count = error_count.clone();

        let mut client = WireframeClient::builder()
            .on_error(move |_err| count_error(&count))
            .connect(addr)
            .await?;

        let request = Envelope::new(42, None, vec![1, 2, 3]);
        let _result: Result<Envelope, ClientError> = client.call_correlated(request).await;

        assert_eq!(
            error_count.load(Ordering::SeqCst),
            1,
            "error hook should be invoked on correlation mismatch"
        );

        drop(client);
        Ok::<(), BodyError>(())
    })
    .await
    .expect("error hook on correlation mismatch");
}

#[tokio::test]
async fn multiple_sequential_calls_get_unique_correlation_ids() {
    let (addr, server) = spawn_envelope_echo_server()
        .await
        .expect("spawn echo server");

    run_guarded(server, async move {
        let mut client = WireframeClient::builder().connect(addr).await?;

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

        drop(client);
        Ok::<(), BodyError>(())
    })
    .await
    .expect("sequential correlated calls");
}

#[rstest]
#[case(vec![])]
#[case(vec![0u8; 100])]
#[case(vec![255u8; 500])]
#[tokio::test]
async fn round_trip_with_various_payload_sizes(#[case] payload: Vec<u8>) {
    let (addr, server) = spawn_envelope_echo_server()
        .await
        .expect("spawn echo server");

    run_guarded(server, async move {
        let mut client = WireframeClient::builder().connect(addr).await?;

        let request = Envelope::new(1, None, payload.clone());
        let response: Envelope = client
            .call_correlated(request)
            .await
            .expect("call should succeed");

        assert_eq!(
            response.into_parts().into_payload(),
            payload.as_slice(),
            "response.into_parts().into_payload() should equal payload.as_slice()"
        );

        drop(client);
        Ok::<(), BodyError>(())
    })
    .await
    .expect("round-trip with the given payload size");
}

#[tokio::test]
async fn receive_envelope_maps_closed_connection_to_transport_wireframe_error() {
    let (mut client, server_stream) = connect_with_guarded_accept()
        .await
        .expect("connect under a guarded accept");

    // Immediately drop the connection.
    drop(server_stream);

    let result: Result<Envelope, ClientError> = client.receive_envelope().await;
    assert!(
        matches!(result, Err(ClientError::Wireframe(WireframeError::Io(_)))),
        "expected transport error mapped to WireframeError::Io"
    );
}
