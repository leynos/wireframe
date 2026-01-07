//! Step definitions for message assembly multiplexing and continuity validation.

use cucumber::{given, then, when};

use crate::world::{MessageAssemblyWorld, TestResult};

// =============================================================================
// Given steps
// =============================================================================

#[given(expr = "a message assembly state with max size {int} and timeout {int} seconds")]
fn given_state(world: &mut MessageAssemblyWorld, max_size: usize, timeout: u64) {
    world.create_state(max_size, timeout);
}

#[given(expr = "a first frame for key {int} with metadata {string} and body {string}")]
fn given_first_frame_with_metadata(
    world: &mut MessageAssemblyWorld,
    key: u64,
    metadata: String,
    body: String,
) {
    world.add_first_frame(key, metadata.into_bytes(), body.into_bytes(), false);
}

#[given(expr = "a first frame for key {int} with body {string}")]
fn given_first_frame(world: &mut MessageAssemblyWorld, key: u64, body: String) {
    world.add_first_frame(key, vec![], body.into_bytes(), false);
}

#[given(expr = "a final first frame for key {int} with body {string}")]
fn given_final_first_frame(world: &mut MessageAssemblyWorld, key: u64, body: String) {
    world.add_first_frame(key, vec![], body.into_bytes(), true);
}

// =============================================================================
// When steps
// =============================================================================

#[when("the first frame is accepted")]
fn when_first_frame_accepted(world: &mut MessageAssemblyWorld) -> TestResult {
    world.accept_first_frame()
}

#[when("the first frame is accepted at time T")]
fn when_first_frame_accepted_at_time_t(world: &mut MessageAssemblyWorld) -> TestResult {
    world.accept_first_frame()
}

#[when("all first frames are accepted")]
fn when_all_first_frames_accepted(world: &mut MessageAssemblyWorld) -> TestResult {
    world.accept_all_first_frames()
}

#[when(expr = "a final continuation for key {int} with sequence {int} and body {string} arrives")]
#[expect(
    clippy::needless_pass_by_value,
    reason = "cucumber hands {string} captures to step functions as owned strings"
)]
fn when_final_continuation(
    world: &mut MessageAssemblyWorld,
    key: u64,
    seq: u32,
    body: String,
) -> TestResult {
    world.accept_continuation(key, Some(seq), body.as_bytes(), true)
}

#[when(expr = "a continuation for key {int} with sequence {int} arrives")]
fn when_continuation_with_seq(world: &mut MessageAssemblyWorld, key: u64, seq: u32) -> TestResult {
    world.accept_continuation(key, Some(seq), b"data", false)
}

#[when(expr = "a continuation for key {int} with body {string} arrives")]
#[expect(
    clippy::needless_pass_by_value,
    reason = "cucumber hands {string} captures to step functions as owned strings"
)]
fn when_continuation_with_body(
    world: &mut MessageAssemblyWorld,
    key: u64,
    body: String,
) -> TestResult {
    world.accept_continuation(key, Some(1), body.as_bytes(), false)
}

#[when(expr = "another first frame for key {int} with body {string} arrives")]
fn when_another_first_frame(world: &mut MessageAssemblyWorld, key: u64, body: String) {
    world.add_first_frame(key, vec![], body.into_bytes(), false);
    // Ignore result, we check error in Then
    let _ = world.accept_first_frame();
}

#[when(expr = "time advances by {int} seconds")]
fn when_time_advances(world: &mut MessageAssemblyWorld, secs: u64) { world.advance_time(secs); }

#[when("expired assemblies are purged")]
fn when_purge_expired(world: &mut MessageAssemblyWorld) -> TestResult { world.purge_expired() }

// =============================================================================
// Then steps
// =============================================================================

#[then("the assembly result is incomplete")]
fn then_result_incomplete(world: &mut MessageAssemblyWorld) -> TestResult {
    if world.last_result_is_incomplete() {
        Ok(())
    } else {
        Err("expected incomplete result".into())
    }
}

#[then(expr = "the assembly completes with body {string}")]
#[expect(
    clippy::needless_pass_by_value,
    reason = "cucumber hands {string} captures to step functions as owned strings"
)]
fn then_completes_with_body(world: &mut MessageAssemblyWorld, body: String) -> TestResult {
    let actual = world
        .last_completed_body()
        .ok_or("expected completed message")?;
    if actual == body.as_bytes() {
        Ok(())
    } else {
        Err(format!(
            "body mismatch: expected {:?}, got {:?}",
            body.as_bytes(),
            actual
        )
        .into())
    }
}

#[then(expr = "key {int} completes with body {string}")]
#[expect(
    clippy::needless_pass_by_value,
    reason = "cucumber hands {string} captures to step functions as owned strings"
)]
fn then_key_completes(world: &mut MessageAssemblyWorld, key: u64, body: String) -> TestResult {
    let actual = world
        .completed_body_for_key(key)
        .ok_or_else(|| format!("expected completed message for key {key}"))?;
    if actual == body.as_bytes() {
        Ok(())
    } else {
        Err(format!(
            "body mismatch for key {key}: expected {:?}, got {:?}",
            body.as_bytes(),
            actual
        )
        .into())
    }
}

#[then(expr = "the buffered count is {int}")]
fn then_buffered_count(world: &mut MessageAssemblyWorld, count: usize) -> TestResult {
    let actual = world.buffered_count();
    if actual == count {
        Ok(())
    } else {
        Err(format!("buffered count mismatch: expected {count}, got {actual}").into())
    }
}

#[then(expr = "the error is sequence mismatch expecting {int} but found {int}")]
fn then_error_sequence_mismatch(
    world: &mut MessageAssemblyWorld,
    expected: u32,
    found: u32,
) -> TestResult {
    if world.is_sequence_mismatch(expected, found) {
        Ok(())
    } else {
        Err(format!(
            "expected sequence mismatch error, got {:?}",
            world.last_error()
        )
        .into())
    }
}

#[then(expr = "the error is duplicate frame for key {int} sequence {int}")]
fn then_error_duplicate_frame(world: &mut MessageAssemblyWorld, key: u64, seq: u32) -> TestResult {
    if world.is_duplicate_frame(key, seq) {
        Ok(())
    } else {
        Err(format!(
            "expected duplicate frame error, got {:?}",
            world.last_error()
        )
        .into())
    }
}

#[then(expr = "the error is missing first frame for key {int}")]
fn then_error_missing_first_frame(world: &mut MessageAssemblyWorld, key: u64) -> TestResult {
    if world.is_missing_first_frame(key) {
        Ok(())
    } else {
        Err(format!(
            "expected missing first frame error, got {:?}",
            world.last_error()
        )
        .into())
    }
}

#[then(expr = "the error is duplicate first frame for key {int}")]
fn then_error_duplicate_first_frame(world: &mut MessageAssemblyWorld, key: u64) -> TestResult {
    if world.is_duplicate_first_frame(key) {
        Ok(())
    } else {
        Err(format!(
            "expected duplicate first frame error, got {:?}",
            world.last_error()
        )
        .into())
    }
}

#[then(expr = "the error is message too large for key {int}")]
fn then_error_message_too_large(world: &mut MessageAssemblyWorld, key: u64) -> TestResult {
    if world.is_message_too_large(key) {
        Ok(())
    } else {
        Err(format!(
            "expected message too large error, got {:?}",
            world.last_error()
        )
        .into())
    }
}

#[then(expr = "key {int} was evicted")]
fn then_key_evicted(world: &mut MessageAssemblyWorld, key: u64) -> TestResult {
    if world.was_evicted(key) {
        Ok(())
    } else {
        Err(format!("expected key {key} to be evicted").into())
    }
}
