//! Unit tests for client tracing spans and per-command timing.
//!
//! The `tracing-test` subscriber captures formatted event output; span names
//! appear as context prefixes. Tests enable per-command timing so an event is
//! emitted within each span, making the span name visible in captured output.

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
    rewind_stream::RewindStream,
    serializer::BincodeSerializer,
};

/// Concrete client type returned by `builder().connect()` in tests.
type TestClient = WireframeClient<BincodeSerializer, RewindStream<tokio::net::TcpStream>>;

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

/// Spawn an echo server, connect a client with the given tracing config,
/// run an async closure against it, then tear down the server.
///
/// The closure takes ownership of the client so that operations such as
/// `close()` (which consumes `self`) work without lifetime gymnastics.
async fn with_echo_client<F, Fut>(config: TracingConfig, f: F)
where
    F: FnOnce(TestClient, std::net::SocketAddr) -> Fut,
    Fut: std::future::Future<Output = ()>,
{
    let (addr, server) = spawn_echo_server().await;
    let client = WireframeClient::builder()
        .tracing_config(config)
        .connect(addr)
        .await
        .expect("connect");
    f(client, addr).await;
    server.abort();
}

/// Return a closure that asserts at least one log line contains
/// `span_name` and every string in `required_fields`. Pass the
/// returned closure to `logs_assert`.
fn span_assertion(
    span_name: &str,
    required_fields: &[&str],
) -> impl Fn(&[&str]) -> Result<(), String> {
    let span = span_name.to_owned();
    let fields: Vec<String> = required_fields.iter().map(|s| (*s).to_owned()).collect();
    move |lines: &[&str]| {
        lines
            .iter()
            .find(|line| line.contains(&span) && fields.iter().all(|f| line.contains(f.as_str())))
            .map(|_| ())
            .ok_or_else(|| format!("{span} not found in:\n{}", lines.join("\n")))
    }
}

#[rstest]
#[traced_test]
#[tokio::test]
async fn connect_emits_span_with_peer_address() {
    let captured_addr = std::sync::OnceLock::new();
    with_echo_client(
        TracingConfig::default().with_connect_timing(true),
        |_client, addr| {
            captured_addr.set(addr.to_string()).expect("set addr");
            async {}
        },
    )
    .await;

    // The peer address is dynamic, so we capture it from the closure.
    let addr_str = captured_addr.get().expect("addr captured");
    logs_assert(span_assertion("client.connect", &[addr_str]));
}

#[rstest]
#[traced_test]
#[tokio::test]
async fn send_emits_span_with_frame_bytes() {
    with_echo_client(
        TracingConfig::default().with_send_timing(true),
        |mut client, _addr| async move {
            let envelope = Envelope::new(1, None, vec![1, 2, 3]);
            client.send(&envelope).await.expect("send");
        },
    )
    .await;

    logs_assert(span_assertion("client.send", &["frame.bytes"]));
}

#[rstest]
#[traced_test]
#[tokio::test]
async fn receive_emits_span_with_result() {
    with_echo_client(
        TracingConfig::default().with_receive_timing(true),
        |mut client, _addr| async move {
            let envelope = Envelope::new(1, None, vec![1, 2, 3]);
            client.send(&envelope).await.expect("send");
            let _response: Envelope = client.receive().await.expect("receive");
        },
    )
    .await;

    logs_assert(span_assertion("client.receive", &[]));
}

#[rstest]
#[traced_test]
#[tokio::test]
async fn call_emits_wrapping_span() {
    with_echo_client(
        TracingConfig::default().with_call_timing(true),
        |mut client, _addr| async move {
            let envelope = Envelope::new(1, None, vec![1, 2, 3]);
            let _response: Envelope = client.call(&envelope).await.expect("call");
        },
    )
    .await;

    logs_assert(span_assertion("client.call", &[]));
}

#[rstest]
#[traced_test]
#[tokio::test]
async fn call_correlated_emits_span_with_correlation_id() {
    with_echo_client(
        TracingConfig::default().with_call_timing(true),
        |mut client, _addr| async move {
            let request = Envelope::new(1, None, vec![1, 2, 3]);
            let _response: Envelope = client
                .call_correlated(request)
                .await
                .expect("call_correlated");
        },
    )
    .await;

    logs_assert(span_assertion(
        "client.call_correlated",
        &["correlation_id"],
    ));
}

#[rstest]
#[traced_test]
#[tokio::test]
async fn send_envelope_emits_span_with_correlation_id_and_frame_bytes() {
    with_echo_client(
        TracingConfig::default().with_send_timing(true),
        |mut client, _addr| async move {
            let envelope = Envelope::new(1, None, vec![1, 2, 3]);
            let _cid = client.send_envelope(envelope).await.expect("send_envelope");
        },
    )
    .await;

    logs_assert(span_assertion(
        "client.send_envelope",
        &["correlation_id", "frame.bytes"],
    ));
}

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
    logs_assert(span_assertion("correlation_id", &[]));
}

#[rstest]
#[traced_test]
#[tokio::test]
async fn close_emits_span() {
    with_echo_client(
        TracingConfig::default().with_close_timing(true),
        |client, _addr| async move {
            client.close().await;
        },
    )
    .await;

    logs_assert(span_assertion("client.close", &[]));
}

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

    logs_assert(span_assertion("client.receive", &[]));
}

#[rstest]
#[traced_test]
#[tokio::test]
async fn timing_disabled_by_default() {
    with_echo_client(TracingConfig::default(), |mut client, _addr| async move {
        let envelope = Envelope::new(1, None, vec![1, 2, 3]);
        let _response: Envelope = client.call(&envelope).await.expect("call");
    })
    .await;

    assert!(
        !logs_contain("elapsed_us"),
        "elapsed_us should not appear when timing is disabled"
    );
}

#[rstest]
#[traced_test]
#[tokio::test]
async fn timing_enabled_emits_elapsed_us_for_send() {
    with_echo_client(
        TracingConfig::default().with_send_timing(true),
        |mut client, _addr| async move {
            let envelope = Envelope::new(1, None, vec![1, 2, 3]);
            client.send(&envelope).await.expect("send");
        },
    )
    .await;

    logs_assert(span_assertion("elapsed_us", &[]));
}

#[rstest]
#[traced_test]
#[tokio::test]
async fn timing_enabled_for_connect() {
    with_echo_client(
        TracingConfig::default().with_connect_timing(true),
        |_client, _addr| async {},
    )
    .await;

    logs_assert(span_assertion("elapsed_us", &[]));
}

#[rstest]
#[traced_test]
#[tokio::test]
async fn all_timing_convenience_enables_all_operations() {
    with_echo_client(
        TracingConfig::default().with_all_timing(true),
        |mut client, _addr| async move {
            let envelope = Envelope::new(1, None, vec![1, 2, 3]);
            let _response: Envelope = client.call(&envelope).await.expect("call");
        },
    )
    .await;

    // At minimum: connect + send + receive + call = 4 timing events.
    logs_assert(|lines: &[&str]| {
        let count = lines.iter().filter(|l| l.contains("elapsed_us")).count();
        if count >= 4 {
            Ok(())
        } else {
            Err(format!("expected >=4 elapsed_us events, found {count}"))
        }
    });
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

#[rstest]
#[traced_test]
#[tokio::test]
async fn default_config_is_backwards_compatible() {
    // No tracing_config() call — uses the default. Verifies no panic
    // occurs and basic operations succeed with default configuration.
    let (addr, server) = spawn_echo_server().await;
    let mut client = WireframeClient::builder()
        .connect(addr)
        .await
        .expect("connect");
    let envelope = Envelope::new(1, None, vec![1, 2, 3]);
    let _response: Envelope = client.call(&envelope).await.expect("call");
    server.abort();
}
