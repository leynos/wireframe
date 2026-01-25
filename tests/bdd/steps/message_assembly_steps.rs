//! Step definitions for message assembly multiplexing and continuity validation.

use rstest_bdd_macros::{given, then, when};
use wireframe::message_assembler::{FrameSequence, MessageKey};

use crate::fixtures::message_assembly::{
    ContinuationFrameParams,
    FirstFrameParams,
    MessageAssemblyWorld,
    TestResult,
};

/// Convert primitive key to domain type at the boundary.
fn to_key(key: u64) -> MessageKey { MessageKey(key) }

/// Convert primitive sequence to domain type at the boundary.
fn to_seq(seq: u32) -> FrameSequence { FrameSequence(seq) }

/// Helper function to reduce duplication in Then step assertions.
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

#[given(
    "a message assembly state with max size {max_size:usize} and timeout {timeout:u64} seconds"
)]
fn given_state(message_assembly_world: &mut MessageAssemblyWorld, max_size: usize, timeout: u64) {
    message_assembly_world.create_state(max_size, timeout);
}

#[given("a first frame for key {key:u64} with metadata {metadata:string} and body {body:string}")]
fn given_first_frame_with_metadata(
    message_assembly_world: &mut MessageAssemblyWorld,
    key: u64,
    metadata: String,
    body: String,
) {
    message_assembly_world.add_first_frame(
        FirstFrameParams::new(to_key(key), body.into_bytes()).with_metadata(metadata.into_bytes()),
    );
}

#[given("a first frame for key {key:u64} with body {body:string}")]
fn given_first_frame(message_assembly_world: &mut MessageAssemblyWorld, key: u64, body: String) {
    message_assembly_world.add_first_frame(FirstFrameParams::new(to_key(key), body.into_bytes()));
}

#[given("a final first frame for key {key:u64} with body {body:string}")]
fn given_final_first_frame(
    message_assembly_world: &mut MessageAssemblyWorld,
    key: u64,
    body: String,
) {
    message_assembly_world
        .add_first_frame(FirstFrameParams::new(to_key(key), body.into_bytes()).final_frame());
}

// =============================================================================
// When steps
// =============================================================================

#[when("the first frame is accepted")]
#[when("the first frame is accepted at time T")]
fn when_first_frame_accepted(message_assembly_world: &mut MessageAssemblyWorld) -> TestResult {
    message_assembly_world.accept_first_frame()
}

#[when("all first frames are accepted")]
fn when_all_first_frames_accepted(message_assembly_world: &mut MessageAssemblyWorld) -> TestResult {
    message_assembly_world.accept_all_first_frames()
}

#[when(
    "a final continuation for key {key:u64} with sequence {sequence:u32} and body {body:string} \
     arrives"
)]
fn when_final_continuation(
    message_assembly_world: &mut MessageAssemblyWorld,
    key: u64,
    sequence: u32,
    body: String,
) -> TestResult {
    message_assembly_world.accept_continuation(
        ContinuationFrameParams::new(to_key(key), body.into_bytes())
            .with_sequence(to_seq(sequence))
            .final_frame(),
    )
}

#[when("a continuation for key {key:u64} with sequence {sequence:u32} arrives")]
fn when_continuation_with_seq(
    message_assembly_world: &mut MessageAssemblyWorld,
    key: u64,
    sequence: u32,
) -> TestResult {
    message_assembly_world.accept_continuation(
        ContinuationFrameParams::new(to_key(key), b"data".to_vec()).with_sequence(to_seq(sequence)),
    )
}

#[when("a continuation for key {key:u64} with body {body:string} arrives")]
fn when_continuation_with_body(
    message_assembly_world: &mut MessageAssemblyWorld,
    key: u64,
    body: String,
) -> TestResult {
    message_assembly_world
        .accept_continuation(ContinuationFrameParams::new(to_key(key), body.into_bytes()))
}

#[when("another first frame for key {key:u64} with body {body:string} arrives")]
fn when_another_first_frame(
    message_assembly_world: &mut MessageAssemblyWorld,
    key: u64,
    body: String,
) -> TestResult {
    message_assembly_world.add_first_frame(FirstFrameParams::new(to_key(key), body.into_bytes()));
    message_assembly_world.accept_first_frame()
}

#[when("time advances by {secs:u64} seconds")]
fn when_time_advances(message_assembly_world: &mut MessageAssemblyWorld, secs: u64) -> TestResult {
    message_assembly_world.advance_time(secs)
}

#[when("expired assemblies are purged")]
fn when_purge_expired(message_assembly_world: &mut MessageAssemblyWorld) -> TestResult {
    message_assembly_world.purge_expired()
}

// =============================================================================
// Then steps
// =============================================================================

#[then("the assembly result is incomplete")]
fn then_result_incomplete(message_assembly_world: &mut MessageAssemblyWorld) -> TestResult {
    assert_condition(
        message_assembly_world.last_result_is_incomplete(),
        "expected incomplete result",
    )
}

#[then("the assembly completes with body {body:string}")]
fn then_completes_with_body(
    message_assembly_world: &mut MessageAssemblyWorld,
    body: String,
) -> TestResult {
    let actual = message_assembly_world
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

#[then("key {key:u64} completes with body {body:string}")]
fn then_key_completes(
    message_assembly_world: &mut MessageAssemblyWorld,
    key: u64,
    body: String,
) -> TestResult {
    let actual = message_assembly_world
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

#[then("the buffered count is {count:usize}")]
fn then_buffered_count(
    message_assembly_world: &mut MessageAssemblyWorld,
    count: usize,
) -> TestResult {
    let actual = message_assembly_world.buffered_count();
    assert_condition(
        actual == count,
        format!("buffered count mismatch: expected {count}, got {actual}"),
    )
}

#[then("the error is sequence mismatch expecting {expected:u32} but found {found:u32}")]
fn then_error_sequence_mismatch(
    message_assembly_world: &mut MessageAssemblyWorld,
    expected: u32,
    found: u32,
) -> TestResult {
    assert_condition(
        message_assembly_world.is_sequence_mismatch(to_seq(expected), to_seq(found)),
        format!(
            "expected sequence mismatch error, got {:?}",
            message_assembly_world.last_error()
        ),
    )
}

#[then("the error is duplicate frame for key {key:u64} sequence {sequence:u32}")]
fn then_error_duplicate_frame(
    message_assembly_world: &mut MessageAssemblyWorld,
    key: u64,
    sequence: u32,
) -> TestResult {
    assert_condition(
        message_assembly_world.is_duplicate_frame(to_key(key), to_seq(sequence)),
        format!(
            "expected duplicate frame error, got {:?}",
            message_assembly_world.last_error()
        ),
    )
}

#[then("the error is missing first frame for key {key:u64}")]
fn then_error_missing_first_frame(
    message_assembly_world: &mut MessageAssemblyWorld,
    key: u64,
) -> TestResult {
    assert_condition(
        message_assembly_world.is_missing_first_frame(to_key(key)),
        format!(
            "expected missing first frame error, got {:?}",
            message_assembly_world.last_error()
        ),
    )
}

#[then("the error is duplicate first frame for key {key:u64}")]
fn then_error_duplicate_first_frame(
    message_assembly_world: &mut MessageAssemblyWorld,
    key: u64,
) -> TestResult {
    assert_condition(
        message_assembly_world.is_duplicate_first_frame(to_key(key)),
        format!(
            "expected duplicate first frame error, got {:?}",
            message_assembly_world.last_error()
        ),
    )
}

#[then("the error is message too large for key {key:u64}")]
fn then_error_message_too_large(
    message_assembly_world: &mut MessageAssemblyWorld,
    key: u64,
) -> TestResult {
    assert_condition(
        message_assembly_world.is_message_too_large(to_key(key)),
        format!(
            "expected message too large error, got {:?}",
            message_assembly_world.last_error()
        ),
    )
}

#[then("key {key:u64} was evicted")]
fn then_key_evicted(message_assembly_world: &mut MessageAssemblyWorld, key: u64) -> TestResult {
    assert_condition(
        message_assembly_world.was_evicted(to_key(key)),
        format!("expected key {key} to be evicted"),
    )
}
