//! Integration tests for the unified codec path.
//!
//! Validates that all outbound frames — handler responses, fragmented
//! payloads — pass through the [`FramePipeline`] before reaching the wire.
//! These tests exercise the codec path end-to-end via
//! `WireframeApp::handle_connection_result` over in-memory duplex streams.
#![cfg(not(loom))]

use std::time::Duration;

use futures::SinkExt;
use rstest::{fixture, rstest};
use tokio::{io::AsyncWriteExt, sync::mpsc, time::timeout};
use tokio_util::codec::{Framed, LengthDelimitedCodec};
use wireframe::{
    app::{Envelope, Packet, WireframeApp},
    fragment::{FragmentationConfig, decode_fragment_payload},
};

#[path = "common/fragment_helpers.rs"]
#[expect(
    dead_code,
    reason = "shared helper module; not all items used by every test binary"
)]
mod fragment_helpers;
#[path = "common/unified_codec_transport.rs"]
mod unified_codec_transport;

use crate::{
    fragment_helpers::{
        CORRELATION,
        ROUTE_ID,
        TestError,
        TestResult,
        build_envelopes,
        fragmentation_config,
        make_handler,
        read_response_payload,
        send_envelopes,
        spawn_app,
    },
    unified_codec_transport::{recv_one, send_one},
};

/// Default buffer capacity used in unified codec tests.
const CAPACITY: usize = 512;

/// Shared harness for unified codec integration tests.
struct UnifiedCodecHarness {
    client: Framed<tokio::io::DuplexStream, LengthDelimitedCodec>,
    server: tokio::task::JoinHandle<std::io::Result<()>>,
    rx: mpsc::UnboundedReceiver<Vec<u8>>,
}

/// Helper to build an echo-style app with optional fragmentation.
fn echo_app(
    config: Option<FragmentationConfig>,
    tx: &mpsc::UnboundedSender<Vec<u8>>,
) -> TestResult<WireframeApp> {
    let handler = make_handler(tx);
    let mut app = WireframeApp::new()?.buffer_capacity(CAPACITY);
    if let Some(c) = config {
        app = app.fragmentation(Some(c));
    }
    Ok(app.route(ROUTE_ID, handler)?)
}

/// Build the unified codec test harness and return client/server test handles.
fn setup_harness(config: Option<FragmentationConfig>) -> TestResult<UnifiedCodecHarness> {
    let (tx, rx) = mpsc::unbounded_channel();
    let app = echo_app(config, &tx)?;
    let (client, server) = spawn_app(app);
    Ok(UnifiedCodecHarness { client, server, rx })
}

// ---------------------------------------------------------------------------
// Test: basic request-response passes through the unified pipeline
// ---------------------------------------------------------------------------

#[fixture]
fn fragmented_harness() -> TestResult<UnifiedCodecHarness> {
    let config = fragmentation_config(CAPACITY)?;
    setup_harness(Some(config))
}

#[rstest]
#[tokio::test]
async fn handler_response_round_trips_through_pipeline() -> TestResult {
    let unfragmented_harness = setup_harness(None)?;
    let UnifiedCodecHarness {
        mut client,
        server,
        mut rx,
    } = unfragmented_harness;

    let payload = vec![1, 2, 3, 4, 5];
    let request = Envelope::new(ROUTE_ID, CORRELATION, payload.clone());

    send_one(&mut client, &request).await?;
    client.get_mut().shutdown().await?;

    // Handler received the payload
    let observed = timeout(Duration::from_secs(1), rx.recv())
        .await?
        .ok_or(TestError::Setup("handler payload missing"))?;
    assert_eq!(observed, payload, "handler should receive original payload");

    // Response arrives back
    let response = recv_one(&mut client).await?;
    let response_payload = response.into_parts().into_payload();
    assert_eq!(
        response_payload, payload,
        "response payload should match request"
    );

    server.await??;
    Ok(())
}

// ---------------------------------------------------------------------------
// Test: fragmentation applies through the unified pipeline
// ---------------------------------------------------------------------------

#[rstest]
#[tokio::test]
async fn fragmented_response_passes_through_pipeline(
    fragmented_harness: TestResult<UnifiedCodecHarness>,
) -> TestResult {
    let config = fragmentation_config(CAPACITY)?;
    let UnifiedCodecHarness {
        mut client,
        server,
        mut rx,
    } = fragmented_harness?;

    // Payload larger than fragment capacity to trigger fragmentation.
    // Must also fragment the *request* so it fits in the codec frame.
    let payload = vec![b'F'; 1_200];
    let request = Envelope::new(ROUTE_ID, CORRELATION, payload.clone());
    let envelopes = build_envelopes(request, &config, true)?;

    send_envelopes(&mut client, &envelopes).await?;
    client.flush().await?;
    client.get_mut().shutdown().await?;

    // Handler received the reassembled payload
    let observed = timeout(Duration::from_secs(1), rx.recv())
        .await?
        .ok_or(TestError::Setup("handler payload missing"))?;
    assert_eq!(observed, payload, "handler should receive original payload");

    // Read fragmented response and reassemble
    let response = read_response_payload(&mut client, &config).await?;
    assert_eq!(
        response, payload,
        "reassembled response should match original payload"
    );

    server.await??;
    Ok(())
}

// ---------------------------------------------------------------------------
// Test: small payload passes through without fragmentation
// ---------------------------------------------------------------------------

#[rstest]
#[tokio::test]
async fn small_payload_passes_unfragmented(
    fragmented_harness: TestResult<UnifiedCodecHarness>,
) -> TestResult {
    let UnifiedCodecHarness {
        mut client,
        server,
        mut rx,
    } = fragmented_harness?;

    let payload = vec![b'S'; 16];
    let request = Envelope::new(ROUTE_ID, CORRELATION, payload.clone());

    send_one(&mut client, &request).await?;
    client.get_mut().shutdown().await?;

    // Handler received the payload
    let observed = timeout(Duration::from_secs(1), rx.recv())
        .await?
        .ok_or(TestError::Setup("handler payload missing"))?;
    assert_eq!(observed, payload);

    // Response should be unfragmented
    let response = recv_one(&mut client).await?;
    let response_payload = response.into_parts().into_payload();

    // Verify no fragment header present
    assert!(
        decode_fragment_payload(&response_payload)?.is_none(),
        "small payload should not be fragmented"
    );
    assert_eq!(response_payload, payload);

    server.await??;
    Ok(())
}

// ---------------------------------------------------------------------------
// Test: multiple sequential requests through the pipeline
// ---------------------------------------------------------------------------

#[rstest]
#[tokio::test]
async fn multiple_sequential_requests_through_pipeline() -> TestResult {
    let unfragmented_harness = setup_harness(None)?;
    let UnifiedCodecHarness {
        mut client,
        server,
        mut rx,
    } = unfragmented_harness;

    let payloads: Vec<Vec<u8>> = (0..5).map(|i| vec![i; 8]).collect();

    for payload in &payloads {
        let request = Envelope::new(ROUTE_ID, CORRELATION, payload.clone());
        send_one(&mut client, &request).await?;
    }
    client.get_mut().shutdown().await?;

    // All handlers should fire
    for expected in &payloads {
        let observed = timeout(Duration::from_secs(1), rx.recv())
            .await?
            .ok_or(TestError::Setup("handler payload missing"))?;
        assert_eq!(&observed, expected);
    }

    // All responses should arrive
    for expected in &payloads {
        let response = recv_one(&mut client).await?;
        let response_payload = response.into_parts().into_payload();
        assert_eq!(&response_payload, expected);
    }

    server.await??;
    Ok(())
}

// ---------------------------------------------------------------------------
// Test: pipeline respects disabled fragmentation
// ---------------------------------------------------------------------------

#[rstest]
#[tokio::test]
async fn pipeline_with_no_fragmentation_passes_large_payload() -> TestResult {
    let unfragmented_harness = setup_harness(None)?;
    let UnifiedCodecHarness {
        mut client,
        server,
        mut rx,
    } = unfragmented_harness;

    let payload = vec![b'L'; 256];
    let request = Envelope::new(ROUTE_ID, CORRELATION, payload.clone());

    send_one(&mut client, &request).await?;
    client.get_mut().shutdown().await?;

    let observed = timeout(Duration::from_secs(1), rx.recv())
        .await?
        .ok_or(TestError::Setup("handler payload missing"))?;
    assert_eq!(observed, payload);

    let response = recv_one(&mut client).await?;
    let response_payload = response.into_parts().into_payload();
    assert_eq!(response_payload, payload);
    assert!(
        decode_fragment_payload(&response_payload)?.is_none(),
        "response should not contain fragment headers when fragmentation is disabled"
    );

    server.await??;
    Ok(())
}
