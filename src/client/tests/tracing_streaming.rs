//! Tracing tests for streaming client operations.
//!
//! These tests verify that `call_streaming` and the response stream emit the
//! expected tracing spans and events. They are separated from the core tracing
//! tests because they depend on the streaming test infrastructure.

use futures::StreamExt;
use rstest::rstest;
use tracing_test::traced_test;

use super::{
    streaming_infra::{
        CorrelationId,
        MessageId,
        Payload,
        TestStreamEnvelope,
        setup_streaming_test,
        spawn_test_server,
    },
    tracing::span_assertion,
};
use crate::client::{TracingConfig, WireframeClient};

#[rstest]
#[traced_test]
#[tokio::test]
async fn call_streaming_emits_span_with_correlation_id() {
    let cid = CorrelationId::new(42);
    let frames = vec![
        TestStreamEnvelope::data(MessageId::new(1), cid, Payload::new(vec![10])),
        TestStreamEnvelope::terminator(cid),
    ];

    let server = spawn_test_server(frames, false).await.expect("server");
    let mut client = WireframeClient::builder()
        .tracing_config(TracingConfig::default().with_streaming_timing(true))
        .connect(server.addr)
        .await
        .expect("connect");

    let request = TestStreamEnvelope::data(MessageId::new(99), cid, Payload::new(vec![]));
    let mut stream = client
        .call_streaming::<TestStreamEnvelope>(request)
        .await
        .expect("call_streaming");

    // Consume the stream — frame events carry the call_streaming span context.
    while let Some(result) = stream.next().await {
        let _frame = result.expect("data frame");
    }

    // With streaming timing enabled, the elapsed_us event fires within
    // the client.call_streaming span, making the span name visible.
    logs_assert(span_assertion("client.call_streaming", &["correlation_id"]));
}

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

    // Only data frames increment the counter; the terminator does not.
    assert_eq!(
        stream.frame_count(),
        3,
        "frame_count should only include successfully decoded data frames"
    );

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
