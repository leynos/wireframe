//! Unit tests for client tracing spans and per-command timing.
//!
//! These tests verify that every instrumented client operation emits the
//! expected tracing spans and structured fields, and that per-command timing
//! events are emitted only when enabled.
//!
//! Tests use `#[traced_test]` from `tracing-test` combined with `rstest` and
//! `tokio::test`. The `tracing-test` subscriber captures formatted output from
//! `FmtSubscriber`. Span names appear in event lines as context prefixes, so
//! tests enable per-command timing to produce an event within each span.

use bytes::Bytes;
use futures::{SinkExt, StreamExt};
use rstest::rstest;
use tokio::net::TcpListener;
use tokio_util::codec::{Framed, LengthDelimitedCodec};
use tracing_test::traced_test;
use wireframe_testing::{ServerMode, process_frame};

use super::streaming_infra::{
    CorrelationId,
    MessageId,
    Payload,
    TestStreamEnvelope,
    setup_streaming_test,
};
use crate::{
    app::Envelope,
    client::{ClientError, TracingConfig, WireframeClient},
};

/// Spawn a test echo server that deserialises envelopes and echoes them back.
async fn spawn_echo_server() -> (std::net::SocketAddr, tokio::task::JoinHandle<()>) {
    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind listener");
    let addr = listener.local_addr().expect("listener addr");

    let handle = tokio::spawn(async move {
        let (stream, _) = listener.accept().await.expect("accept client");
        let mut framed = Framed::new(stream, LengthDelimitedCodec::new());

        while let Some(Ok(bytes)) = framed.next().await {
            let Some(response_bytes) = process_frame(ServerMode::Echo, &bytes) else {
                break;
            };
            if framed.send(Bytes::from(response_bytes)).await.is_err() {
                break;
            }
        }
    });

    (addr, handle)
}

// ---------------------------------------------------------------------------
// Connect — timing event appears inside client.connect span
// ---------------------------------------------------------------------------

#[rstest]
#[traced_test]
#[tokio::test]
async fn connect_emits_span_with_peer_address() {
    let (addr, server) = spawn_echo_server().await;
    let _client = WireframeClient::builder()
        .tracing_config(TracingConfig::default().with_connect_timing(true))
        .connect(addr)
        .await
        .expect("connect");
    server.abort();

    logs_assert(|lines: &[&str]| {
        lines
            .iter()
            .find(|line| line.contains("client.connect") && line.contains(&addr.to_string()))
            .map(|_| ())
            .ok_or_else(|| {
                format!(
                    "client.connect span with peer address not found in:\n{}",
                    lines.join("\n")
                )
            })
    });
}

// ---------------------------------------------------------------------------
// Send — timing event appears inside client.send span
// ---------------------------------------------------------------------------

#[rstest]
#[traced_test]
#[tokio::test]
async fn send_emits_span_with_frame_bytes() {
    let (addr, server) = spawn_echo_server().await;
    let mut client = WireframeClient::builder()
        .tracing_config(TracingConfig::default().with_send_timing(true))
        .connect(addr)
        .await
        .expect("connect");

    let envelope = Envelope::new(1, None, vec![1, 2, 3]);
    client.send(&envelope).await.expect("send");
    server.abort();

    logs_assert(|lines: &[&str]| {
        lines
            .iter()
            .find(|line| line.contains("client.send") && line.contains("frame.bytes"))
            .map(|_| ())
            .ok_or_else(|| {
                format!(
                    "client.send span with frame.bytes not found in:\n{}",
                    lines.join("\n")
                )
            })
    });
}

// ---------------------------------------------------------------------------
// Receive — timing event appears inside client.receive span
// ---------------------------------------------------------------------------

#[rstest]
#[traced_test]
#[tokio::test]
async fn receive_emits_span_with_result() {
    let (addr, server) = spawn_echo_server().await;
    let mut client = WireframeClient::builder()
        .tracing_config(TracingConfig::default().with_receive_timing(true))
        .connect(addr)
        .await
        .expect("connect");

    let envelope = Envelope::new(1, None, vec![1, 2, 3]);
    client.send(&envelope).await.expect("send");
    let _response: Envelope = client.receive().await.expect("receive");
    server.abort();

    logs_assert(|lines: &[&str]| {
        lines
            .iter()
            .find(|line| line.contains("client.receive"))
            .map(|_| ())
            .ok_or_else(|| format!("client.receive span not found in:\n{}", lines.join("\n")))
    });
}

// ---------------------------------------------------------------------------
// Call — timing event appears inside client.call span
// ---------------------------------------------------------------------------

#[rstest]
#[traced_test]
#[tokio::test]
async fn call_emits_wrapping_span() {
    let (addr, server) = spawn_echo_server().await;
    let mut client = WireframeClient::builder()
        .tracing_config(TracingConfig::default().with_call_timing(true))
        .connect(addr)
        .await
        .expect("connect");

    let envelope = Envelope::new(1, None, vec![1, 2, 3]);
    let _response: Envelope = client.call(&envelope).await.expect("call");
    server.abort();

    logs_assert(|lines: &[&str]| {
        lines
            .iter()
            .find(|line| line.contains("client.call"))
            .map(|_| ())
            .ok_or_else(|| format!("client.call span not found in:\n{}", lines.join("\n")))
    });
}

// ---------------------------------------------------------------------------
// Call correlated — timing event appears inside client.call_correlated span
// ---------------------------------------------------------------------------

#[rstest]
#[traced_test]
#[tokio::test]
async fn call_correlated_emits_span_with_correlation_id() {
    let (addr, server) = spawn_echo_server().await;
    let mut client = WireframeClient::builder()
        .tracing_config(TracingConfig::default().with_call_timing(true))
        .connect(addr)
        .await
        .expect("connect");

    let request = Envelope::new(1, None, vec![1, 2, 3]);
    let _response: Envelope = client
        .call_correlated(request)
        .await
        .expect("call_correlated");
    server.abort();

    logs_assert(|lines: &[&str]| {
        lines
            .iter()
            .find(|line| line.contains("client.call_correlated") && line.contains("correlation_id"))
            .map(|_| ())
            .ok_or_else(|| {
                format!(
                    "client.call_correlated span with correlation_id not found in:\n{}",
                    lines.join("\n")
                )
            })
    });
}

// ---------------------------------------------------------------------------
// Send envelope — timing event appears inside client.send_envelope span
// ---------------------------------------------------------------------------

#[rstest]
#[traced_test]
#[tokio::test]
async fn send_envelope_emits_span_with_correlation_id_and_frame_bytes() {
    let (addr, server) = spawn_echo_server().await;
    let mut client = WireframeClient::builder()
        .tracing_config(TracingConfig::default().with_send_timing(true))
        .connect(addr)
        .await
        .expect("connect");

    let envelope = Envelope::new(1, None, vec![1, 2, 3]);
    let _cid = client.send_envelope(envelope).await.expect("send_envelope");
    server.abort();

    logs_assert(|lines: &[&str]| {
        lines
            .iter()
            .find(|line| {
                line.contains("client.send_envelope")
                    && line.contains("correlation_id")
                    && line.contains("frame.bytes")
            })
            .map(|_| ())
            .ok_or_else(|| {
                format!(
                    "client.send_envelope span with correlation_id and frame.bytes not found \
                     in:\n{}",
                    lines.join("\n")
                )
            })
    });
}

// ---------------------------------------------------------------------------
// Streaming — tracing::debug! events in poll_next/process_frame
// ---------------------------------------------------------------------------

#[rstest]
#[traced_test]
#[tokio::test]
async fn call_streaming_emits_span_with_correlation_id() {
    let cid = CorrelationId::new(42);
    let frames = vec![
        TestStreamEnvelope::data(MessageId::new(1), cid, Payload::new(vec![10])),
        TestStreamEnvelope::terminator(cid),
    ];

    let (mut client, _server) = setup_streaming_test(frames).await.expect("setup");

    let request = TestStreamEnvelope::data(MessageId::new(99), cid, Payload::new(vec![]));
    let mut stream = client
        .call_streaming::<TestStreamEnvelope>(request)
        .await
        .expect("call_streaming");

    // Consume the stream — frame events carry the call_streaming span context.
    while let Some(result) = stream.next().await {
        let _frame = result.expect("data frame");
    }

    // The per-frame "stream frame received" event is emitted with
    // correlation_id in the enclosing call_streaming span context.
    logs_assert(|lines: &[&str]| {
        lines
            .iter()
            .find(|line| line.contains("correlation_id"))
            .map(|_| ())
            .ok_or_else(|| {
                format!(
                    "correlation_id not found in streaming output:\n{}",
                    lines.join("\n")
                )
            })
    });
}

// ---------------------------------------------------------------------------
// Close — timing event appears inside client.close span
// ---------------------------------------------------------------------------

#[rstest]
#[traced_test]
#[tokio::test]
async fn close_emits_span() {
    let (addr, server) = spawn_echo_server().await;
    let client = WireframeClient::builder()
        .tracing_config(TracingConfig::default().with_close_timing(true))
        .connect(addr)
        .await
        .expect("connect");
    server.abort();

    client.close().await;

    logs_assert(|lines: &[&str]| {
        lines
            .iter()
            .find(|line| line.contains("client.close"))
            .map(|_| ())
            .ok_or_else(|| format!("client.close span not found in:\n{}", lines.join("\n")))
    });
}

// ---------------------------------------------------------------------------
// Error path — receive timing emits within client.receive span
// ---------------------------------------------------------------------------

#[rstest]
#[traced_test]
#[tokio::test]
async fn receive_error_records_result_err() {
    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind listener");
    let addr = listener.local_addr().expect("listener addr");

    let accept = tokio::spawn(async move {
        let (stream, _) = listener.accept().await.expect("accept");
        drop(stream);
    });

    let mut client = WireframeClient::builder()
        .tracing_config(TracingConfig::default().with_receive_timing(true))
        .connect(addr)
        .await
        .expect("connect");

    accept.await.expect("join accept");

    let result: Result<Envelope, ClientError> = client.receive().await;
    assert!(result.is_err(), "receive should fail after disconnect");

    logs_assert(|lines: &[&str]| {
        lines
            .iter()
            .find(|line| line.contains("client.receive"))
            .map(|_| ())
            .ok_or_else(|| {
                format!(
                    "client.receive span not found on error path in:\n{}",
                    lines.join("\n")
                )
            })
    });
}

// ---------------------------------------------------------------------------
// Timing
// ---------------------------------------------------------------------------

#[rstest]
#[traced_test]
#[tokio::test]
async fn timing_disabled_by_default() {
    let (addr, server) = spawn_echo_server().await;
    let mut client = WireframeClient::builder()
        .tracing_config(TracingConfig::default())
        .connect(addr)
        .await
        .expect("connect");

    let envelope = Envelope::new(1, None, vec![1, 2, 3]);
    let _response: Envelope = client.call(&envelope).await.expect("call");
    server.abort();

    assert!(
        !logs_contain("elapsed_us"),
        "elapsed_us should not appear when timing is disabled"
    );
}

#[rstest]
#[traced_test]
#[tokio::test]
async fn timing_enabled_emits_elapsed_us_for_send() {
    let (addr, server) = spawn_echo_server().await;
    let mut client = WireframeClient::builder()
        .tracing_config(TracingConfig::default().with_send_timing(true))
        .connect(addr)
        .await
        .expect("connect");

    let envelope = Envelope::new(1, None, vec![1, 2, 3]);
    client.send(&envelope).await.expect("send");
    server.abort();

    logs_assert(|lines: &[&str]| {
        lines
            .iter()
            .find(|line| line.contains("elapsed_us"))
            .map(|_| ())
            .ok_or_else(|| "elapsed_us not found with send timing enabled".to_string())
    });
}

#[rstest]
#[traced_test]
#[tokio::test]
async fn timing_enabled_for_connect() {
    let (addr, server) = spawn_echo_server().await;
    let _client = WireframeClient::builder()
        .tracing_config(TracingConfig::default().with_connect_timing(true))
        .connect(addr)
        .await
        .expect("connect");
    server.abort();

    logs_assert(|lines: &[&str]| {
        lines
            .iter()
            .find(|line| line.contains("elapsed_us"))
            .map(|_| ())
            .ok_or_else(|| "elapsed_us not found with connect timing enabled".to_string())
    });
}

#[rstest]
#[traced_test]
#[tokio::test]
async fn all_timing_convenience_enables_all_operations() {
    let (addr, server) = spawn_echo_server().await;
    let mut client = WireframeClient::builder()
        .tracing_config(TracingConfig::default().with_all_timing(true))
        .connect(addr)
        .await
        .expect("connect");

    let envelope = Envelope::new(1, None, vec![1, 2, 3]);
    let _response: Envelope = client.call(&envelope).await.expect("call");
    server.abort();

    logs_assert(|lines: &[&str]| {
        let count = lines
            .iter()
            .filter(|line| line.contains("elapsed_us"))
            .count();
        // At minimum: connect + send + receive + call = 4 timing events.
        if count >= 4 {
            Ok(())
        } else {
            Err(format!(
                "expected at least 4 elapsed_us events, found {count}"
            ))
        }
    });
}

// ---------------------------------------------------------------------------
// Streaming events
// ---------------------------------------------------------------------------

#[rstest]
#[traced_test]
#[tokio::test]
async fn response_stream_emits_frame_events() {
    let cid = CorrelationId::new(42);
    let frames = vec![
        TestStreamEnvelope::data(MessageId::new(1), cid, Payload::new(vec![10])),
        TestStreamEnvelope::data(MessageId::new(2), cid, Payload::new(vec![20])),
        TestStreamEnvelope::data(MessageId::new(3), cid, Payload::new(vec![30])),
        TestStreamEnvelope::terminator(cid),
    ];

    let (mut client, _server) = setup_streaming_test(frames).await.expect("setup");

    let request = TestStreamEnvelope::data(MessageId::new(99), cid, Payload::new(vec![]));
    let mut stream = client
        .call_streaming::<TestStreamEnvelope>(request)
        .await
        .expect("call_streaming");

    while let Some(result) = stream.next().await {
        let _frame = result.expect("data frame");
    }

    logs_assert(|lines: &[&str]| {
        let frame_events = lines
            .iter()
            .filter(|line| line.contains("stream frame received"))
            .count();
        if frame_events < 3 {
            return Err(format!(
                "expected at least 3 'stream frame received' events, found {frame_events}"
            ));
        }
        lines
            .iter()
            .find(|line| line.contains("stream terminated"))
            .map(|_| ())
            .ok_or_else(|| "'stream terminated' event not found".to_string())
    });
}

// ---------------------------------------------------------------------------
// Default config backwards compatibility
// ---------------------------------------------------------------------------

#[rstest]
#[traced_test]
#[tokio::test]
async fn default_config_is_backwards_compatible() {
    let (addr, server) = spawn_echo_server().await;

    // No tracing_config() call — uses the default.
    let mut client = WireframeClient::builder()
        .connect(addr)
        .await
        .expect("connect");

    let envelope = Envelope::new(1, None, vec![1, 2, 3]);
    let _response: Envelope = client.call(&envelope).await.expect("call");
    server.abort();

    // With default config and no timing enabled, no events are emitted
    // within spans. The test verifies no panic occurs and basic operations
    // succeed — backwards compatibility is preserved.
}
