//! Step definitions for message assembly multiplexing and continuity validation.

use cucumber::{given, then, when};
use wireframe::message_assembler::{FrameSequence, MessageKey};

use crate::world::{ContinuationFrameParams, FirstFrameParams, MessageAssemblyWorld, TestResult};

/// Convert primitive key to domain type at the boundary.
fn to_key(key: u64) -> MessageKey { MessageKey(key) }

/// Convert primitive sequence to domain type at the boundary.
fn to_seq(seq: u32) -> FrameSequence { FrameSequence(seq) }

/// Helper function to reduce duplication in Then step assertions
fn assert_condition(condition: bool, error_msg: impl Into<String>) -> TestResult {
    if condition {
        Ok(())
    } else {
        Err(error_msg.into().into())
    }
}

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
    world.add_first_frame(
        FirstFrameParams::new(to_key(key), body.into_bytes()).with_metadata(metadata.into_bytes()),
    );
}

#[given(expr = "a first frame for key {int} with body {string}")]
fn given_first_frame(world: &mut MessageAssemblyWorld, key: u64, body: String) {
    world.add_first_frame(FirstFrameParams::new(to_key(key), body.into_bytes()));
}

#[given(expr = "a final first frame for key {int} with body {string}")]
fn given_final_first_frame(world: &mut MessageAssemblyWorld, key: u64, body: String) {
    world.add_first_frame(FirstFrameParams::new(to_key(key), body.into_bytes()).final_frame());
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
fn when_final_continuation(
    world: &mut MessageAssemblyWorld,
    key: u64,
    seq: u32,
    body: String,
) -> TestResult {
    world.accept_continuation(
        ContinuationFrameParams::new(to_key(key), body.into_bytes())
            .with_sequence(to_seq(seq))
            .final_frame(),
    )
}

#[when(expr = "a continuation for key {int} with sequence {int} arrives")]
fn when_continuation_with_seq(world: &mut MessageAssemblyWorld, key: u64, seq: u32) -> TestResult {
    world.accept_continuation(
        ContinuationFrameParams::new(to_key(key), b"data".to_vec()).with_sequence(to_seq(seq)),
    )
}

#[when(expr = "a continuation for key {int} with body {string} arrives")]
fn when_continuation_with_body(
    world: &mut MessageAssemblyWorld,
    key: u64,
    body: String,
) -> TestResult {
    world.accept_continuation(ContinuationFrameParams::new(to_key(key), body.into_bytes()))
}

#[when(expr = "another first frame for key {int} with body {string} arrives")]
fn when_another_first_frame(
    world: &mut MessageAssemblyWorld,
    key: u64,
    body: String,
) -> TestResult {
    world.add_first_frame(FirstFrameParams::new(to_key(key), body.into_bytes()));
    world.accept_first_frame()
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
    assert_condition(
        world.last_result_is_incomplete(),
        "expected incomplete result",
    )
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
    assert_condition(
        actual == body.as_bytes(),
        format!(
            "body mismatch: expected {:?}, got {:?}",
            body.as_bytes(),
            actual
        ),
    )
}

#[then(expr = "key {int} completes with body {string}")]
#[expect(
    clippy::needless_pass_by_value,
    reason = "cucumber hands {string} captures to step functions as owned strings"
)]
fn then_key_completes(world: &mut MessageAssemblyWorld, key: u64, body: String) -> TestResult {
    let actual = world
        .completed_body_for_key(to_key(key))
        .ok_or_else(|| format!("expected completed message for key {key}"))?;
    assert_condition(
        actual == body.as_bytes(),
        format!(
            "body mismatch for key {key}: expected {:?}, got {:?}",
            body.as_bytes(),
            actual
        ),
    )
}

#[then(expr = "the buffered count is {int}")]
fn then_buffered_count(world: &mut MessageAssemblyWorld, count: usize) -> TestResult {
    let actual = world.buffered_count();
    assert_condition(
        actual == count,
        format!("buffered count mismatch: expected {count}, got {actual}"),
    )
}

#[then(expr = "the error is sequence mismatch expecting {int} but found {int}")]
fn then_error_sequence_mismatch(
    world: &mut MessageAssemblyWorld,
    expected: u32,
    found: u32,
) -> TestResult {
    assert_condition(
        world.is_sequence_mismatch(to_seq(expected), to_seq(found)),
        format!(
            "expected sequence mismatch error, got {:?}",
            world.last_error()
        ),
    )
}

#[then(expr = "the error is duplicate frame for key {int} sequence {int}")]
fn then_error_duplicate_frame(world: &mut MessageAssemblyWorld, key: u64, seq: u32) -> TestResult {
    assert_condition(
        world.is_duplicate_frame(to_key(key), to_seq(seq)),
        format!(
            "expected duplicate frame error, got {:?}",
            world.last_error()
        ),
    )
}

#[then(expr = "the error is missing first frame for key {int}")]
fn then_error_missing_first_frame(world: &mut MessageAssemblyWorld, key: u64) -> TestResult {
    assert_condition(
        world.is_missing_first_frame(to_key(key)),
        format!(
            "expected missing first frame error, got {:?}",
            world.last_error()
        ),
    )
}

#[then(expr = "the error is duplicate first frame for key {int}")]
fn then_error_duplicate_first_frame(world: &mut MessageAssemblyWorld, key: u64) -> TestResult {
    assert_condition(
        world.is_duplicate_first_frame(to_key(key)),
        format!(
            "expected duplicate first frame error, got {:?}",
            world.last_error()
        ),
    )
}

#[then(expr = "the error is message too large for key {int}")]
fn then_error_message_too_large(world: &mut MessageAssemblyWorld, key: u64) -> TestResult {
    assert_condition(
        world.is_message_too_large(to_key(key)),
        format!(
            "expected message too large error, got {:?}",
            world.last_error()
        ),
    )
}

#[then(expr = "key {int} was evicted")]
fn then_key_evicted(world: &mut MessageAssemblyWorld, key: u64) -> TestResult {
    assert_condition(
        world.was_evicted(to_key(key)),
        format!("expected key {key} to be evicted"),
    )
}
