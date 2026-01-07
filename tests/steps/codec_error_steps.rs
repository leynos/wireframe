//! Steps for codec error taxonomy behavioural tests.
//!
//! These steps exercise real codec operations to verify error handling
//! and recovery policy behaviour.

use cucumber::{given, then, when};

use crate::world::{CodecErrorWorld, TestResult};

// =============================================================================
// Given steps
// =============================================================================

#[given(expr = "a wireframe server with default codec")]
fn given_server_default(world: &mut CodecErrorWorld) { world.setup_default_codec(); }

#[given(expr = "a wireframe server with max frame length {int} bytes")]
fn given_server_max_frame(world: &mut CodecErrorWorld, max_len: usize) {
    world.setup_codec_with_max_length(max_len);
}

#[given(expr = "a codec error of type {word} with variant {word}")]
fn given_error_type_variant(
    world: &mut CodecErrorWorld,
    error_type: String,
    variant: String,
) -> TestResult {
    let error_type = error_type.into_boxed_str();
    let variant = variant.into_boxed_str();
    world.set_error_type(error_type.as_ref())?;
    match error_type.as_ref() {
        "framing" => world.set_framing_variant(variant.as_ref())?,
        "eof" => world.set_eof_variant(variant.as_ref())?,
        _ => {}
    }
    Ok(())
}

// =============================================================================
// When steps - Real codec operations
// =============================================================================

#[when("a client connects and sends a complete frame")]
fn when_client_sends_complete(world: &mut CodecErrorWorld) -> TestResult {
    // Send a small payload that fits within the frame limit
    world.send_complete_frame(&[1, 2, 3, 4])
}

#[when("the client closes the connection cleanly")]
fn when_client_closes_clean(world: &mut CodecErrorWorld) -> TestResult {
    // Decode the complete frame, then call decode_eof on empty buffer
    world.decode_eof_clean_close()
}

#[when("a client connects and sends partial frame data")]
fn when_client_sends_partial(world: &mut CodecErrorWorld) {
    // Send header indicating 100 bytes, but no payload
    world.send_partial_frame_header_only();
}

#[when("the client closes the connection abruptly")]
fn when_client_closes_abrupt(world: &mut CodecErrorWorld) -> TestResult {
    // Call decode_eof with partial data in buffer
    world.decode_eof_with_partial_data()
}

#[when(expr = "a client sends a frame larger than {int} bytes")]
fn when_client_sends_oversized(world: &mut CodecErrorWorld, max: usize) -> TestResult {
    // Attempt to encode a payload larger than max_frame_length
    // Add 1 to exceed the limit
    world.encode_oversized_frame(max + 1)
}

// =============================================================================
// Then steps - Verification
// =============================================================================

#[then("the server detects a clean EOF")]
fn then_server_clean_eof(world: &mut CodecErrorWorld) -> TestResult { world.verify_clean_eof() }

#[then("the server detects a mid-frame EOF with partial data")]
fn then_server_mid_frame_eof(world: &mut CodecErrorWorld) -> TestResult {
    world.verify_mid_frame_eof()
}

#[then("the server rejects the frame with an oversized error")]
fn then_server_oversized(world: &mut CodecErrorWorld) -> TestResult {
    world.verify_oversized_error()
}

#[then(expr = "the default recovery policy is {word}")]
fn then_recovery_policy(world: &mut CodecErrorWorld, policy: String) -> TestResult {
    let policy = policy.into_boxed_str();
    world.verify_recovery_policy(policy.as_ref())
}
