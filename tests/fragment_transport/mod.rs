//! Integration tests for transport-level fragmentation and reassembly.
//!
//! Tests are organized into submodules by concern:
//! - Round-trip tests (this file)
//! - Rejection tests (`fragment_transport/rejection.rs`)
//! - Eviction tests (`fragment_transport/eviction.rs`)
#![cfg(not(loom))]

use std::{io, time::Duration};

use futures::{SinkExt, StreamExt};
use rstest::{fixture, rstest};
use tokio::{io::AsyncWriteExt, sync::mpsc, time::timeout};
use tokio_util::codec::{Framed, LengthDelimitedCodec};
use wireframe::{
    app::{Envelope, Packet, WireframeApp},
    fragment::{FragmentationConfig, Fragmenter, decode_fragment_payload},
    serializer::{BincodeSerializer, Serializer},
};

#[path = "subtests.rs"]
mod fragment_transport;

#[path = "../common/fragment_helpers/mod.rs"]
mod fragment_helpers;

use crate::fragment_helpers::{
    CORRELATION,
    FramedResponse,
    ROUTE_ID,
    TestError,
    TestResult,
    assert_handler_observed,
    build_envelopes,
    fragment_envelope,
    fragmentation_config,
    make_app,
    make_handler,
    read_framed_response,
    read_reassembled_response,
    read_response_payload,
    send_envelopes,
    spawn_app,
};

/// Buffer capacity shared by the tests that drive [`Fragmenter`] directly.
const BUFFER_CAPACITY: usize = 512;

/// Scaffold for tests that build their own fragment stream.
///
/// Grouping these five values keeps the fallible setup in one place; a test
/// that needs different fragmentation settings should build its own app
/// instead of widening this fixture.
struct FragmentHarness {
    /// Fragmentation settings matching [`BUFFER_CAPACITY`].
    config: FragmentationConfig,
    /// Client end of the duplex connection.
    client: Framed<tokio::io::DuplexStream, LengthDelimitedCodec>,
    /// Server task driving the application.
    server: tokio::task::JoinHandle<io::Result<()>>,
    /// Payloads observed by the handler.
    rx: mpsc::UnboundedReceiver<Vec<u8>>,
}

/// Build a fragmentation-enabled harness at [`BUFFER_CAPACITY`].
#[fixture]
fn fragment_harness() -> TestResult<FragmentHarness> {
    let config = fragmentation_config(BUFFER_CAPACITY)?;
    let (tx, rx) = mpsc::unbounded_channel();
    let app = make_app(BUFFER_CAPACITY, config, &tx)?;
    let (client, server) = spawn_app(app);
    Ok(FragmentHarness {
        config,
        client,
        server,
        rx,
    })
}

/// Common helper for round-trip fragmentation tests.
/// Returns the framed response for additional test-specific assertions.
async fn run_round_trip_test(
    buffer_capacity: usize,
    payload: Vec<u8>,
    should_fragment: bool,
) -> TestResult<FramedResponse> {
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
    if response.payload != payload {
        return Err(TestError::Assertion(format!(
            "response payload mismatch: expected {payload:?}, got {:?}",
            response.payload
        )));
    }

    server.await??;

    Ok(response)
}

#[tokio::test]
async fn fragmented_request_and_response_round_trip() -> TestResult {
    let buffer_capacity = 512;
    let payload = vec![b'Z'; 1_200];

    // `read_framed_response` accepts an unfragmented response, so without this
    // assertion a regression that stopped fragmenting the response would still
    // satisfy the reassembly and payload checks inside `run_round_trip_test`.
    let response = run_round_trip_test(buffer_capacity, payload, true).await?;
    if response.frame_count <= 1 {
        return Err(TestError::Assertion(format!(
            "a large response must cross multiple frames, saw {}",
            response.frame_count
        )));
    }

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
    if decode_fragment_payload(&response.payload)?.is_some() {
        return Err(TestError::Assertion(
            "small payload should pass through unfragmented".to_string(),
        ));
    }
    if response.frame_count != 1 {
        return Err(TestError::Assertion(format!(
            "a payload within the fragment cap should cross exactly one frame, saw {}",
            response.frame_count
        )));
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

    // One byte past the cap proves the threshold is inert while fragmentation
    // is disabled, rather than merely confirming a half-sized payload passes.
    let payload_len = config
        .fragment_payload_cap
        .get()
        .checked_add(1)
        .ok_or(TestError::Setup("fragment payload cap overflow"))?;
    let payload = vec![b'X'; payload_len];
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
        read_framed_response(&mut client, &config),
    )
    .await??;
    if decode_fragment_payload(&response.payload)?.is_some() {
        return Err(TestError::Assertion(
            "expected no fragmentation when fragmentation is disabled".to_string(),
        ));
    }
    if response.frame_count != 1 {
        return Err(TestError::Assertion(format!(
            "a payload past the cap must still cross one frame when fragmentation is off, saw {}",
            response.frame_count
        )));
    }

    server.await??;

    Ok(())
}

#[tokio::test]
async fn fragmentation_is_opt_in_by_default() -> TestResult {
    let capacity = 512;
    let payload = vec![b'd'; 128];
    let (tx, mut rx) = mpsc::unbounded_channel();
    let handler = make_handler(&tx);

    let app: WireframeApp = WireframeApp::new()?
        .buffer_capacity(capacity)
        .route(ROUTE_ID, handler)?;
    let (mut client, server) = spawn_app(app);

    let request = Envelope::new(ROUTE_ID, CORRELATION, payload.clone());
    let serializer = BincodeSerializer;
    let bytes = serializer.serialize(&request)?;
    client.send(bytes.into()).await?;
    client.get_mut().shutdown().await?;

    assert_handler_observed(&mut rx, &payload).await?;
    let response_bytes = timeout(Duration::from_secs(1), client.next())
        .await?
        .ok_or(TestError::Setup("response frame missing"))??;
    let (response_env, _) = serializer.deserialize::<Envelope>(&response_bytes)?;
    let response_payload = response_env.into_parts().into_payload();
    if decode_fragment_payload(&response_payload)?.is_some() {
        return Err(TestError::Assertion(
            "default app should keep fragmentation disabled".to_string(),
        ));
    }

    server.await??;

    Ok(())
}

#[rstest]
#[tokio::test]
async fn duplicate_fragment_is_suppressed_and_reassembles(
    fragment_harness: TestResult<FragmentHarness>,
) -> TestResult {
    let FragmentHarness {
        config,
        mut client,
        server,
        mut rx,
    } = fragment_harness?;
    let fragmenter = Fragmenter::new(config.fragment_payload_cap);

    let payload = vec![b'D'; 800];
    let request = Envelope::new(ROUTE_ID, CORRELATION, payload.clone());
    let mut fragments = fragment_envelope(&request, &fragmenter)?;
    let duplicate = fragments
        .first()
        .cloned()
        .ok_or(TestError::Setup("fragmenter produced no fragments"))?;
    fragments.insert(1, duplicate);

    send_envelopes(&mut client, &fragments).await?;
    client.flush().await?;
    client.get_mut().shutdown().await?;

    assert_handler_observed(&mut rx, &payload).await?;
    if let Ok(Some(_)) = timeout(Duration::from_millis(200), rx.recv()).await {
        return Err(TestError::Assertion(
            "duplicate suppression should prevent duplicate handler delivery".to_string(),
        ));
    }

    let response = read_reassembled_response(&mut client, &config).await?;
    if response != payload {
        return Err(TestError::Assertion(
            "reassembled payload mismatch after duplicate suppression".to_string(),
        ));
    }

    server.await??;
    Ok(())
}

#[rstest]
#[tokio::test]
async fn interleaved_fragment_streams_reassemble_independently(
    fragment_harness: TestResult<FragmentHarness>,
) -> TestResult {
    let FragmentHarness {
        config,
        mut client,
        server,
        mut rx,
    } = fragment_harness?;
    let fragmenter = Fragmenter::new(config.fragment_payload_cap);

    let payload_a = vec![b'A'; 800];
    let payload_b = vec![b'B'; 900];
    let request_a = Envelope::new(ROUTE_ID, Some(101), payload_a.clone());
    let request_b = Envelope::new(ROUTE_ID, Some(202), payload_b.clone());
    let fragments_a = fragment_envelope(&request_a, &fragmenter)?;
    let fragments_b = fragment_envelope(&request_b, &fragmenter)?;

    let mut interleaved = Vec::with_capacity(fragments_a.len() + fragments_b.len());
    let mut idx = 0usize;
    while idx < fragments_a.len() || idx < fragments_b.len() {
        if let Some(fragment) = fragments_a.get(idx) {
            interleaved.push(fragment.clone());
        }
        if let Some(fragment) = fragments_b.get(idx) {
            interleaved.push(fragment.clone());
        }
        idx += 1;
    }

    send_envelopes(&mut client, &interleaved).await?;
    client.flush().await?;
    client.get_mut().shutdown().await?;

    let response_a = timeout(
        Duration::from_secs(1),
        read_reassembled_response(&mut client, &config),
    )
    .await??;
    let response_b = timeout(
        Duration::from_secs(1),
        read_reassembled_response(&mut client, &config),
    )
    .await??;
    let mut observed_responses = vec![response_a, response_b];
    observed_responses.sort();
    let mut expected_payloads = vec![payload_a.clone(), payload_b.clone()];
    expected_payloads.sort();
    if observed_responses != expected_payloads {
        return Err(TestError::Assertion(
            "interleaved responses were not reassembled correctly".to_string(),
        ));
    }

    let first = timeout(Duration::from_secs(1), rx.recv())
        .await?
        .ok_or(TestError::Setup("first handler payload missing"))?;
    let second = timeout(Duration::from_secs(1), rx.recv())
        .await?
        .ok_or(TestError::Setup("second handler payload missing"))?;
    let mut observed_requests = vec![first, second];
    observed_requests.sort();
    if observed_requests != expected_payloads {
        return Err(TestError::Assertion(
            "interleaved reassembly delivered unexpected handler payloads".to_string(),
        ));
    }

    server.await??;
    Ok(())
}
