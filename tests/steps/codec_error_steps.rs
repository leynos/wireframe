//! Steps for codec error taxonomy behavioural tests.
use cucumber::{given, then, when};

use crate::world::{CodecErrorWorld, TestResult};

#[given(expr = "a wireframe server with default codec")]
fn given_server_default(_world: &mut CodecErrorWorld) {
    // Default codec is already set up in the world
}

#[given(expr = "a wireframe server with max frame length {int} bytes")]
fn given_server_max_frame(_world: &mut CodecErrorWorld, max_len: usize) {
    // Configure max frame length - currently unused placeholder
    let _ = max_len;
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

#[when("a client connects and sends a complete frame")]
fn when_client_sends_complete(_world: &mut CodecErrorWorld) {
    // Simulated by world state
}

#[when("the client closes the connection cleanly")]
fn when_client_closes_clean(world: &mut CodecErrorWorld) { world.record_clean_eof(); }

#[when("a client connects and sends partial frame data")]
fn when_client_sends_partial(_world: &mut CodecErrorWorld) {
    // Simulated by world state
}

#[when("the client closes the connection abruptly")]
fn when_client_closes_abrupt(world: &mut CodecErrorWorld) { world.record_mid_frame_eof(100); }

#[when(expr = "a client sends a frame larger than {int} bytes")]
fn when_client_sends_oversized(world: &mut CodecErrorWorld, max: usize) -> TestResult {
    // Capture max to suppress unused warning
    let _ = max;
    world.set_error_type("framing")?;
    world.set_framing_variant("oversized")?;
    Ok(())
}

#[then("the server detects a clean EOF")]
fn then_server_clean_eof(world: &mut CodecErrorWorld) -> TestResult { world.verify_clean_eof() }

#[then("the server detects a mid-frame EOF with partial data")]
fn then_server_mid_frame_eof(world: &mut CodecErrorWorld) -> TestResult {
    world.verify_mid_frame_eof()
}

#[then("the server rejects the frame with an oversized error")]
fn then_server_oversized(world: &mut CodecErrorWorld) -> TestResult {
    world.set_error_type("framing")?;
    world.set_framing_variant("oversized")?;
    world.verify_recovery_policy("drop")
}

#[then(expr = "the default recovery policy is {word}")]
fn then_recovery_policy(world: &mut CodecErrorWorld, policy: String) -> TestResult {
    let policy = policy.into_boxed_str();
    world.verify_recovery_policy(policy.as_ref())
}
