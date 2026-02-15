//! Integration tests for the unified codec path.
//!
//! Validates that all outbound frames — handler responses, fragmented
//! payloads — pass through the [`FramePipeline`] before reaching the wire.
//! These tests exercise the codec path end-to-end via
//! `WireframeApp::handle_connection_result` over in-memory duplex streams.
#![cfg(not(loom))]

use std::time::Duration;

use futures::{SinkExt, StreamExt};
use rstest::rstest;
use tokio::{io::AsyncWriteExt, sync::mpsc, time::timeout};
use tokio_util::codec::{Framed, LengthDelimitedCodec};
use wireframe::{
    Serializer,
    app::{Envelope, Packet, WireframeApp},
    fragment::{FragmentationConfig, decode_fragment_payload},
    serializer::BincodeSerializer,
};

#[path = "common/fragment_helpers.rs"]
#[expect(
    dead_code,
    reason = "shared helper module; not all items used by every test binary"
)]
mod fragment_helpers;

use crate::fragment_helpers::{
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
};

/// Default buffer capacity used in unified codec tests.
const CAPACITY: usize = 512;

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

/// Serialize and send a single envelope to the server.
async fn send_one(
    client: &mut Framed<tokio::io::DuplexStream, LengthDelimitedCodec>,
    envelope: &Envelope,
) -> TestResult {
    let serializer = BincodeSerializer;
    let bytes = serializer.serialize(envelope)?;
    client.send(bytes.into()).await?;
    Ok(())
}

/// Read a single response envelope from the client.
async fn recv_one(
    client: &mut Framed<tokio::io::DuplexStream, LengthDelimitedCodec>,
) -> TestResult<Envelope> {
    let serializer = BincodeSerializer;
    let frame = timeout(Duration::from_secs(2), client.next())
        .await?
        .ok_or(TestError::Setup("response frame missing"))??;
    let (env, _) = serializer.deserialize::<Envelope>(&frame)?;
    Ok(env)
}

// ---------------------------------------------------------------------------
// Test: basic request-response passes through the unified pipeline
// ---------------------------------------------------------------------------

#[rstest]
#[tokio::test]
async fn handler_response_round_trips_through_pipeline() -> TestResult {
    let (tx, mut rx) = mpsc::unbounded_channel();
    let app = echo_app(None, &tx)?;
    let (mut client, server) = spawn_app(app);

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
async fn fragmented_response_passes_through_pipeline() -> TestResult {
    let config = fragmentation_config(CAPACITY)?;
    let (tx, mut rx) = mpsc::unbounded_channel();
    let app = echo_app(Some(config), &tx)?;
    let (mut client, server) = spawn_app(app);

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
async fn small_payload_passes_unfragmented() -> TestResult {
    let config = fragmentation_config(CAPACITY)?;
    let (tx, mut rx) = mpsc::unbounded_channel();
    let app = echo_app(Some(config), &tx)?;
    let (mut client, server) = spawn_app(app);

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
    let (tx, mut rx) = mpsc::unbounded_channel();
    let app = echo_app(None, &tx)?;
    let (mut client, server) = spawn_app(app);

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
    let (tx, mut rx) = mpsc::unbounded_channel();
    let app = echo_app(None, &tx)?;
    let (mut client, server) = spawn_app(app);

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
