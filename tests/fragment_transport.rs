//! Integration tests for transport-level fragmentation and reassembly.
//!
//! Tests are organized into submodules by concern:
//! - Round-trip tests (this file)
//! - Rejection tests (`fragment_transport/rejection.rs`)
//! - Eviction tests (`fragment_transport/eviction.rs`)
#![cfg(not(loom))]

use std::time::Duration;

use futures::SinkExt;
use tokio::{io::AsyncWriteExt, sync::mpsc, time::timeout};
use wireframe::{
    Serializer,
    app::{Envelope, WireframeApp},
    fragment::decode_fragment_payload,
    serializer::BincodeSerializer,
};

#[path = "fragment_transport/mod.rs"]
mod fragment_transport;

#[path = "common/fragment_helpers.rs"]
mod fragment_helpers;

use crate::fragment_helpers::{
    CORRELATION,
    ROUTE_ID,
    TestError,
    TestResult,
    assert_handler_observed,
    build_envelopes,
    fragmentation_config,
    make_app,
    make_handler,
    read_reassembled_response,
    read_response_payload,
    send_envelopes,
    spawn_app,
};

/// Common helper for round-trip fragmentation tests.
/// Returns the response payload for additional test-specific assertions.
async fn run_round_trip_test(
    buffer_capacity: usize,
    payload: Vec<u8>,
    should_fragment: bool,
) -> TestResult<Vec<u8>> {
    let config = fragmentation_config(buffer_capacity)?;
    let (tx, mut rx) = mpsc::unbounded_channel();
    let app = make_app(buffer_capacity, config, &tx)?;
    let (mut client, server) = spawn_app(app);

    let request = Envelope::new(ROUTE_ID, CORRELATION, payload.clone());

    let envelopes = build_envelopes(request, &config, should_fragment)?;

    send_envelopes(&mut client, &envelopes).await?;
    client.flush().await?;

    assert_handler_observed(&mut rx, &payload).await?;
    client.get_mut().shutdown().await?;
    let response = read_response_payload(&mut client, &config).await?;
    if response != payload {
        return Err(TestError::Assertion(format!(
            "response payload mismatch: expected {payload:?}, got {response:?}"
        )));
    }

    server.await??;

    Ok(response)
}

#[tokio::test]
async fn fragmented_request_and_response_round_trip() -> TestResult {
    let buffer_capacity = 512;
    let payload = vec![b'Z'; 1_200];
    run_round_trip_test(buffer_capacity, payload, true).await?;
    Ok(())
}

#[tokio::test]
async fn unfragmented_request_and_response_round_trip() -> TestResult {
    let buffer_capacity = 512;
    let config = fragmentation_config(buffer_capacity)?;
    let cap = config.fragment_payload_cap.get();
    let payload_len = cap.saturating_sub(8).max(1);
    let payload = vec![b's'; payload_len];

    let response = run_round_trip_test(buffer_capacity, payload, false).await?;
    if decode_fragment_payload(&response)?.is_some() {
        return Err(TestError::Assertion(
            "small payload should pass through unfragmented".to_string(),
        ));
    }

    Ok(())
}

#[tokio::test]
async fn fragmentation_can_be_disabled_via_public_api() -> TestResult {
    let capacity = 1024;
    let (tx, mut rx) = mpsc::unbounded_channel();
    let config = fragmentation_config(capacity)?;

    let handler = make_handler(&tx);

    let app: WireframeApp = WireframeApp::new()?
        .buffer_capacity(capacity)
        .fragmentation(None)
        .route(ROUTE_ID, handler)?;

    let (mut client, server) = spawn_app(app);

    let half_capacity = capacity
        .checked_div(2)
        .ok_or(TestError::Setup("capacity must be at least two"))?;
    let payload = vec![b'X'; half_capacity];
    let request = Envelope::new(ROUTE_ID, CORRELATION, payload.clone());
    let serializer = BincodeSerializer;
    let bytes = serializer.serialize(&request)?;
    client.send(bytes.into()).await?;

    let observed = timeout(Duration::from_secs(1), rx.recv())
        .await?
        .ok_or(TestError::Setup("handler payload missing"))?;
    if observed != payload {
        return Err(TestError::Assertion(format!(
            "observed payload mismatch: expected {payload:?}, got {observed:?}"
        )));
    }

    client.get_mut().shutdown().await?;
    let response = timeout(
        Duration::from_secs(1),
        read_reassembled_response(&mut client, &config),
    )
    .await??;
    if decode_fragment_payload(&response)?.is_some() {
        return Err(TestError::Assertion(
            "expected no fragmentation when fragmentation is disabled".to_string(),
        ));
    }

    server.await??;

    Ok(())
}
