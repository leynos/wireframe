//! Step definitions for message assembly multiplexing and continuity validation.

use std::{fmt::Debug, str::FromStr};

use rstest_bdd_macros::{given, then, when};
use wireframe::message_assembler::{FrameSequence, MessageKey};

use crate::fixtures::message_assembly::{
    ContinuationFrameParams,
    FirstFrameParams,
    MessageAssemblyWorld,
    TestResult,
};

/// Wrapper for message key parameters in BDD steps.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MessageKeyParam(pub u64);

impl FromStr for MessageKeyParam {
    type Err = std::num::ParseIntError;

    fn from_str(s: &str) -> Result<Self, Self::Err> { s.parse::<u64>().map(MessageKeyParam) }
}

impl MessageKeyParam {
    pub fn to_key(self) -> MessageKey { MessageKey(self.0) }
}

/// Wrapper for sequence number parameters in BDD steps.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SequenceParam(pub u32);

impl FromStr for SequenceParam {
    type Err = std::num::ParseIntError;

    fn from_str(s: &str) -> Result<Self, Self::Err> { s.parse::<u32>().map(SequenceParam) }
}

impl SequenceParam {
    pub fn to_seq(self) -> FrameSequence { FrameSequence(self.0) }
}

/// Wrapper for count/size parameters in BDD steps.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CountParam(pub usize);

impl FromStr for CountParam {
    type Err = std::num::ParseIntError;

    fn from_str(s: &str) -> Result<Self, Self::Err> { s.parse::<usize>().map(CountParam) }
}

/// Wrapper for timeout duration parameters in BDD steps.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TimeoutParam(pub u64);

impl FromStr for TimeoutParam {
    type Err = std::num::ParseIntError;

    fn from_str(s: &str) -> Result<Self, Self::Err> { s.parse::<u64>().map(TimeoutParam) }
}

/// Convert primitive key to domain type at the boundary.
fn to_key(key: u64) -> MessageKey { MessageKey(key) }

/// Convert primitive sequence to domain type at the boundary.
fn to_seq(seq: u32) -> FrameSequence { FrameSequence(seq) }

/// Configuration for message assembly state initialisation.
#[derive(Debug, Clone)]
pub struct AssemblyConfig {
    pub max_message_size: usize,
    pub timeout_seconds: u64,
}

impl AssemblyConfig {
    pub fn new(max_message_size: usize, timeout_seconds: u64) -> Self {
        Self {
            max_message_size,
            timeout_seconds,
        }
    }
}

/// Frame identification combining key and optional sequence.
#[derive(Debug, Clone, Copy)]
pub struct FrameId {
    pub key: MessageKey,
    pub sequence: FrameSequence,
}

impl FrameId {
    pub fn new(key: u64, sequence: u32) -> Self {
        Self {
            key: to_key(key),
            sequence: to_seq(sequence),
        }
    }

    pub fn with_key(key: u64) -> Self {
        Self {
            key: to_key(key),
            sequence: FrameSequence(0),
        }
    }
}

/// Helper function to reduce duplication in Then step assertions.
fn assert_condition(condition: bool, error_msg: impl Into<String>) -> TestResult {
    if condition {
        Ok(())
    } else {
        Err(error_msg.into().into())
    }
}

fn assert_error<F>(
    world: &MessageAssemblyWorld,
    check: F,
    description: impl Into<String>,
) -> TestResult
where
    F: FnOnce(&MessageAssemblyWorld) -> bool,
{
    assert_condition(
        check(world),
        format!("{}; got {:?}", description.into(), world.last_error()),
    )
}

fn assert_equals<T: PartialEq + Debug>(
    actual: &T,
    expected: &T,
    context: impl Into<String>,
) -> TestResult {
    assert_condition(
        actual == expected,
        format!(
            "{}: expected {:?}, got {:?}",
            context.into(),
            expected,
            actual
        ),
    )
}

// =============================================================================
// Given steps
// =============================================================================

#[given(
    "a message assembly state with max size {max_size:CountParam} and timeout \
     {timeout:TimeoutParam} seconds"
)]
fn given_state(
    message_assembly_world: &mut MessageAssemblyWorld,
    max_size: CountParam,
    timeout: TimeoutParam,
) {
    let config = AssemblyConfig::new(max_size.0, timeout.0);
    message_assembly_world.create_state(config.max_message_size, config.timeout_seconds);
}

#[given(
    "a first frame for key {key:MessageKeyParam} with metadata {metadata:string} and body \
     {body:string}"
)]
fn given_first_frame_with_metadata(
    message_assembly_world: &mut MessageAssemblyWorld,
    key: MessageKeyParam,
    metadata: String,
    body: String,
) {
    message_assembly_world.add_first_frame(
        FirstFrameParams::new(key.to_key(), body.into_bytes()).with_metadata(metadata.into_bytes()),
    );
}

#[given("a first frame for key {key:MessageKeyParam} with body {body:string}")]
fn given_first_frame(
    message_assembly_world: &mut MessageAssemblyWorld,
    key: MessageKeyParam,
    body: String,
) {
    message_assembly_world.add_first_frame(FirstFrameParams::new(key.to_key(), body.into_bytes()));
}

#[given("a final first frame for key {key:MessageKeyParam} with body {body:string}")]
fn given_final_first_frame(
    message_assembly_world: &mut MessageAssemblyWorld,
    key: MessageKeyParam,
    body: String,
) {
    message_assembly_world
        .add_first_frame(FirstFrameParams::new(key.to_key(), body.into_bytes()).final_frame());
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
    "a final continuation for key {key:MessageKeyParam} with sequence {sequence:SequenceParam} \
     and body {body:string} arrives"
)]
fn when_final_continuation(
    message_assembly_world: &mut MessageAssemblyWorld,
    key: MessageKeyParam,
    sequence: SequenceParam,
    body: String,
) -> TestResult {
    message_assembly_world.accept_continuation(
        ContinuationFrameParams::new(key.to_key(), body.into_bytes())
            .with_sequence(sequence.to_seq())
            .final_frame(),
    )
}

#[when(
    "a continuation for key {key:MessageKeyParam} with sequence {sequence:SequenceParam} arrives"
)]
fn when_continuation_with_seq(
    message_assembly_world: &mut MessageAssemblyWorld,
    key: MessageKeyParam,
    sequence: SequenceParam,
) -> TestResult {
    message_assembly_world.accept_continuation(
        ContinuationFrameParams::new(key.to_key(), b"data".to_vec())
            .with_sequence(sequence.to_seq()),
    )
}

#[when("a continuation for key {key:MessageKeyParam} with body {body:string} arrives")]
fn when_continuation_with_body(
    message_assembly_world: &mut MessageAssemblyWorld,
    key: MessageKeyParam,
    body: String,
) -> TestResult {
    message_assembly_world.accept_continuation(ContinuationFrameParams::new(
        key.to_key(),
        body.into_bytes(),
    ))
}

#[when("another first frame for key {key:MessageKeyParam} with body {body:string} arrives")]
fn when_another_first_frame(
    message_assembly_world: &mut MessageAssemblyWorld,
    key: MessageKeyParam,
    body: String,
) -> TestResult {
    message_assembly_world.add_first_frame(FirstFrameParams::new(key.to_key(), body.into_bytes()));
    message_assembly_world.accept_first_frame()
}

#[when("time advances by {secs:TimeoutParam} seconds")]
fn when_time_advances(
    message_assembly_world: &mut MessageAssemblyWorld,
    secs: TimeoutParam,
) -> TestResult {
    message_assembly_world.advance_time(secs.0)
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
    assert_equals(&actual, &body.as_bytes(), "body mismatch")
}

#[then("key {key:MessageKeyParam} completes with body {body:string}")]
fn then_key_completes(
    message_assembly_world: &mut MessageAssemblyWorld,
    key: MessageKeyParam,
    body: String,
) -> TestResult {
    let actual = message_assembly_world
        .completed_body_for_key(key.to_key())
        .ok_or_else(|| format!("expected completed message for key {}", key.0))?;
    assert_equals(
        &actual,
        &body.as_bytes(),
        format!("body mismatch for key {}", key.0),
    )
}

#[then("the buffered count is {count:CountParam}")]
fn then_buffered_count(
    message_assembly_world: &mut MessageAssemblyWorld,
    count: CountParam,
) -> TestResult {
    let actual = message_assembly_world.buffered_count();
    assert_equals(&actual, &count.0, "buffered count mismatch")
}

#[then(
    "the error is sequence mismatch expecting {expected:SequenceParam} but found \
     {found:SequenceParam}"
)]
fn then_error_sequence_mismatch(
    message_assembly_world: &mut MessageAssemblyWorld,
    expected: SequenceParam,
    found: SequenceParam,
) -> TestResult {
    assert_error(
        message_assembly_world,
        |world| world.is_sequence_mismatch(expected.to_seq(), found.to_seq()),
        "expected sequence mismatch error",
    )
}

#[then(
    "the error is duplicate frame for key {key:MessageKeyParam} sequence {sequence:SequenceParam}"
)]
fn then_error_duplicate_frame(
    message_assembly_world: &mut MessageAssemblyWorld,
    key: MessageKeyParam,
    sequence: SequenceParam,
) -> TestResult {
    let frame_id = FrameId::new(key.0, sequence.0);
    assert_error(
        message_assembly_world,
        |world| world.is_duplicate_frame(frame_id),
        "expected duplicate frame error",
    )
}

#[then("the error is missing first frame for key {key:MessageKeyParam}")]
fn then_error_missing_first_frame(
    message_assembly_world: &mut MessageAssemblyWorld,
    key: MessageKeyParam,
) -> TestResult {
    let frame_id = FrameId::with_key(key.0);
    assert_error(
        message_assembly_world,
        |world| world.is_missing_first_frame(frame_id.key),
        "expected missing first frame error",
    )
}

#[then("the error is duplicate first frame for key {key:MessageKeyParam}")]
fn then_error_duplicate_first_frame(
    message_assembly_world: &mut MessageAssemblyWorld,
    key: MessageKeyParam,
) -> TestResult {
    assert_error(
        message_assembly_world,
        |world| world.is_duplicate_first_frame(key.to_key()),
        "expected duplicate first frame error",
    )
}

#[then("the error is message too large for key {key:MessageKeyParam}")]
fn then_error_message_too_large(
    message_assembly_world: &mut MessageAssemblyWorld,
    key: MessageKeyParam,
) -> TestResult {
    assert_error(
        message_assembly_world,
        |world| world.is_message_too_large(key.to_key()),
        "expected message too large error",
    )
}

#[then("key {key:MessageKeyParam} was evicted")]
fn then_key_evicted(
    message_assembly_world: &mut MessageAssemblyWorld,
    key: MessageKeyParam,
) -> TestResult {
    assert_error(
        message_assembly_world,
        |world| world.was_evicted(key.to_key()),
        format!("expected key {} to be evicted", key.0),
    )
}
