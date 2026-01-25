//! Step definitions for codec error taxonomy behavioural tests.
//!
//! These steps exercise real codec operations to verify error handling
//! and recovery policy behaviour.

use rstest_bdd_macros::{given, then, when};

use crate::fixtures::codec_error::{CodecErrorWorld, TestResult};

// =============================================================================
// Given steps
// =============================================================================

#[given("a wireframe server with default codec")]
fn given_server_default(codec_error_world: &mut CodecErrorWorld) {
    codec_error_world.setup_default_codec();
}

#[given("a wireframe server with max frame length {max_len:usize} bytes")]
fn given_server_max_frame(codec_error_world: &mut CodecErrorWorld, max_len: usize) {
    codec_error_world.setup_codec_with_max_length(max_len);
}

#[given("a codec error of type {error_type} with variant {variant}")]
fn given_error_type_variant(
    codec_error_world: &mut CodecErrorWorld,
    error_type: String,
    variant: String,
) -> TestResult {
    codec_error_world.set_error_type(&error_type)?;
    match error_type.as_str() {
        "framing" => codec_error_world.set_framing_variant(&variant)?,
        "eof" => codec_error_world.set_eof_variant(&variant)?,
        "protocol" | "io" => {}
        _ => return Err(format!("unknown error type: {error_type}").into()),
    }
    Ok(())
}

// =============================================================================
// When steps - Real codec operations
// =============================================================================

#[when("a client connects and sends a complete frame")]
fn when_client_sends_complete(codec_error_world: &mut CodecErrorWorld) -> TestResult {
    // Send a small payload that fits within the frame limit
    codec_error_world.send_complete_frame(&[1, 2, 3, 4])
}

#[when("the client closes the connection cleanly")]
fn when_client_closes_clean(codec_error_world: &mut CodecErrorWorld) -> TestResult {
    // Decode the complete frame, then call decode_eof on empty buffer
    codec_error_world.decode_eof_clean_close()
}

#[when("a client connects and sends partial frame data")]
fn when_client_sends_partial(codec_error_world: &mut CodecErrorWorld) {
    // Send header indicating 100 bytes, but no payload
    codec_error_world.send_partial_frame_header_only();
}

#[when("the client closes the connection abruptly")]
fn when_client_closes_abrupt(codec_error_world: &mut CodecErrorWorld) -> TestResult {
    // Call decode_eof with partial data in buffer
    codec_error_world.decode_eof_with_partial_data()
}

#[when("a client sends a frame larger than {max_len:usize} bytes")]
fn when_client_sends_oversized(
    codec_error_world: &mut CodecErrorWorld,
    max_len: usize,
) -> TestResult {
    // Attempt to encode a payload larger than max_frame_length
    // Add 1 to exceed the limit
    codec_error_world.encode_oversized_frame(max_len + 1)
}

// =============================================================================
// Then steps - Verification
// =============================================================================

#[then("the server detects a clean EOF")]
fn then_server_clean_eof(codec_error_world: &mut CodecErrorWorld) -> TestResult {
    codec_error_world.verify_clean_eof()
}

#[then("the server detects a mid-frame EOF with partial data")]
fn then_server_mid_frame_eof(codec_error_world: &mut CodecErrorWorld) -> TestResult {
    codec_error_world.verify_incomplete_eof()
}

#[then("the server rejects the frame with an oversized error")]
fn then_server_oversized(codec_error_world: &mut CodecErrorWorld) -> TestResult {
    codec_error_world.verify_oversized_error()
}

#[then("the default recovery policy is {policy}")]
fn then_recovery_policy(codec_error_world: &mut CodecErrorWorld, policy: String) -> TestResult {
    codec_error_world.verify_recovery_policy(&policy)
}
