//! Step definitions for the unified codec pipeline behavioural tests.
//!
//! Steps are synchronous; async fixture methods are driven via
//! `Runtime::new().block_on(...)`.

use rstest_bdd_macros::{given, then, when};

use crate::fixtures::unified_codec::{TestResult, UnifiedCodecWorld};

// ---------------------------------------------------------------------------
// Given
// ---------------------------------------------------------------------------

#[given("a wireframe echo server with a buffer capacity of {cap:usize} bytes")]
fn given_echo_server(unified_codec_world: &mut UnifiedCodecWorld, cap: usize) -> TestResult {
    let rt = tokio::runtime::Runtime::new()?;
    rt.block_on(unified_codec_world.start_server(cap, false))
}

#[given(
    "a wireframe echo server with a buffer capacity of {cap:usize} bytes and fragmentation enabled"
)]
fn given_echo_server_fragmented(
    unified_codec_world: &mut UnifiedCodecWorld,
    cap: usize,
) -> TestResult {
    let rt = tokio::runtime::Runtime::new()?;
    rt.block_on(unified_codec_world.start_server(cap, true))
}

// ---------------------------------------------------------------------------
// When
// ---------------------------------------------------------------------------

#[when("the client sends a {size:usize}-byte payload")]
fn when_client_sends_payload(
    unified_codec_world: &mut UnifiedCodecWorld,
    size: usize,
) -> TestResult {
    let rt = tokio::runtime::Runtime::new()?;
    rt.block_on(unified_codec_world.send_payload(size))
}

#[when("the client sends a fragmented {size:usize}-byte payload")]
fn when_client_sends_fragmented(
    unified_codec_world: &mut UnifiedCodecWorld,
    size: usize,
) -> TestResult {
    let rt = tokio::runtime::Runtime::new()?;
    rt.block_on(unified_codec_world.send_fragmented_payload(size))
}

#[when("the client sends {count:usize} sequential {size:usize}-byte payloads")]
fn when_client_sends_sequential(
    unified_codec_world: &mut UnifiedCodecWorld,
    count: usize,
    size: usize,
) -> TestResult {
    let rt = tokio::runtime::Runtime::new()?;
    rt.block_on(unified_codec_world.send_sequential_payloads(count, size))
}

// ---------------------------------------------------------------------------
// Then
// ---------------------------------------------------------------------------

#[then("the handler receives the original payload")]
fn then_handler_receives_payload(unified_codec_world: &mut UnifiedCodecWorld) -> TestResult {
    let rt = tokio::runtime::Runtime::new()?;
    rt.block_on(unified_codec_world.collect_handler_payloads())?;
    unified_codec_world.verify_handler_payloads()
}

#[then("the handler receives the reassembled payload")]
fn then_handler_receives_reassembled(unified_codec_world: &mut UnifiedCodecWorld) -> TestResult {
    let rt = tokio::runtime::Runtime::new()?;
    rt.block_on(unified_codec_world.collect_handler_payloads())?;
    unified_codec_world.verify_handler_payloads()
}

#[then("the handler receives all {count:usize} payloads in order")]
fn then_handler_receives_all(
    unified_codec_world: &mut UnifiedCodecWorld,
    count: usize,
) -> TestResult {
    let rt = tokio::runtime::Runtime::new()?;
    rt.block_on(unified_codec_world.collect_handler_payloads())?;
    let observed = &unified_codec_world.handler_observed;
    if observed.len() != count {
        return Err(format!("expected {count} handler payloads, got {}", observed.len()).into());
    }
    unified_codec_world.verify_handler_payloads()
}

#[then("the client receives a response matching the original payload")]
fn then_client_receives_response(unified_codec_world: &mut UnifiedCodecWorld) -> TestResult {
    let rt = tokio::runtime::Runtime::new()?;
    rt.block_on(unified_codec_world.collect_single_response())?;
    unified_codec_world.verify_response_payloads()?;
    rt.block_on(unified_codec_world.await_server())
}

#[then("the client receives a fragmented response matching the original payload")]
fn then_client_receives_fragmented(unified_codec_world: &mut UnifiedCodecWorld) -> TestResult {
    let rt = tokio::runtime::Runtime::new()?;
    rt.block_on(unified_codec_world.collect_fragmented_response())?;
    unified_codec_world.verify_response_payloads()?;
    rt.block_on(unified_codec_world.await_server())
}

#[then("the client receives an unfragmented response matching the original payload")]
fn then_client_receives_unfragmented(unified_codec_world: &mut UnifiedCodecWorld) -> TestResult {
    let rt = tokio::runtime::Runtime::new()?;
    rt.block_on(unified_codec_world.collect_single_response())?;
    unified_codec_world.verify_unfragmented()?;
    unified_codec_world.verify_response_payloads()?;
    rt.block_on(unified_codec_world.await_server())
}

#[then("the client receives {count:usize} responses matching the original payloads")]
fn then_client_receives_sequential(
    unified_codec_world: &mut UnifiedCodecWorld,
    count: usize,
) -> TestResult {
    let rt = tokio::runtime::Runtime::new()?;
    rt.block_on(unified_codec_world.collect_sequential_responses(count))?;
    unified_codec_world.verify_response_payloads()?;
    rt.block_on(unified_codec_world.await_server())
}
