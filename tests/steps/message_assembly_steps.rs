//! Step definitions for message assembly multiplexing and continuity validation.

use std::str::FromStr;

use rstest_bdd_macros::{given, then, when};
use wireframe::message_assembler::{FrameSequence, MessageKey};
use wireframe_testing::reassembly::MessageAssemblyErrorExpectation;

use crate::fixtures::message_assembly::{
    AssemblyConfig,
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

// =============================================================================
// Given steps
// =============================================================================

#[rustfmt::skip]
#[given("a message assembly state with max size {max_size:CountParam} and timeout {timeout:TimeoutParam} seconds")]
fn given_state(
    message_assembly_world: &mut MessageAssemblyWorld,
    max_size: CountParam,
    timeout: TimeoutParam,
) {
    let config = AssemblyConfig::new(max_size.0, timeout.0);
    message_assembly_world.create_state(config);
}

/// The frame attributes a Gherkin phrase may add beyond key and body.
#[derive(Clone, Debug, Default)]
struct FrameOptions {
    metadata: Option<Vec<u8>>,
    total: Option<usize>,
    is_last: bool,
}

impl FrameOptions {
    /// Attach `metadata` to the frame.
    fn metadata(self, metadata: Vec<u8>) -> Self {
        Self {
            metadata: Some(metadata),
            ..self
        }
    }

    /// Declare a total body length.
    fn total(self, total: CountParam) -> Self {
        Self {
            total: Some(total.0),
            ..self
        }
    }

    /// Mark the frame as completing its message.
    fn final_frame(self) -> Self {
        Self {
            is_last: true,
            ..self
        }
    }

    /// Apply these attributes to `params`.
    fn apply_to(self, params: FirstFrameParams) -> FirstFrameParams {
        let with_metadata = match self.metadata {
            Some(metadata) => params.with_metadata(metadata),
            None => params,
        };
        let with_total = match self.total {
            Some(total) => with_metadata.with_total(total),
            None => with_metadata,
        };
        if self.is_last {
            with_total.final_frame()
        } else {
            with_total
        }
    }
}

/// Record the single first frame the scenario is currently describing.
fn record_first_frame(
    world: &mut MessageAssemblyWorld,
    key: MessageKeyParam,
    body: &str,
    options: FrameOptions,
) {
    world.add_first_frame(options.apply_to(FirstFrameParams::new(
        key.to_key(),
        body.as_bytes().to_vec(),
    )));
}

#[rustfmt::skip]
#[given("a first frame for key {key:MessageKeyParam} with metadata {metadata:string} and body {body:string}")]
fn given_first_frame_with_metadata(
    message_assembly_world: &mut MessageAssemblyWorld,
    key: MessageKeyParam,
    metadata: String,
    body: String,
) {
    let options = FrameOptions::default().metadata(metadata.into_bytes());
    record_first_frame(message_assembly_world, key, &body, options);
}

#[given("a first frame for key {key:MessageKeyParam} with body {body:string}")]
fn given_first_frame(
    message_assembly_world: &mut MessageAssemblyWorld,
    key: MessageKeyParam,
    body: String,
) {
    record_first_frame(message_assembly_world, key, &body, FrameOptions::default());
}

#[rustfmt::skip]
#[given("a first frame for key {key:MessageKeyParam} with body {body:string} declaring total {total:CountParam}")]
fn given_first_frame_with_total(
    message_assembly_world: &mut MessageAssemblyWorld,
    key: MessageKeyParam,
    body: String,
    total: CountParam,
) {
    let options = FrameOptions::default().total(total);
    record_first_frame(message_assembly_world, key, &body, options);
}

#[rustfmt::skip]
#[given("a final first frame for key {key:MessageKeyParam} with body {body:string} declaring total {total:CountParam}")]
fn given_final_first_frame_with_total(
    message_assembly_world: &mut MessageAssemblyWorld,
    key: MessageKeyParam,
    body: String,
    total: CountParam,
) {
    let options = FrameOptions::default().total(total).final_frame();
    record_first_frame(message_assembly_world, key, &body, options);
}

#[given("a final first frame for key {key:MessageKeyParam} with body {body:string}")]
fn given_final_first_frame(
    message_assembly_world: &mut MessageAssemblyWorld,
    key: MessageKeyParam,
    body: String,
) {
    let options = FrameOptions::default().final_frame();
    record_first_frame(message_assembly_world, key, &body, options);
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

#[rustfmt::skip]
#[when("a final continuation for key {key:MessageKeyParam} with sequence {sequence:SequenceParam} and body {body:string} arrives")]
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
    message_assembly_world.assert_result_incomplete()
}

#[then("the assembly completes with body {body:string}")]
fn then_completes_with_body(
    message_assembly_world: &mut MessageAssemblyWorld,
    body: String,
) -> TestResult {
    message_assembly_world.assert_completed_body(body.as_bytes())
}

#[then("key {key:MessageKeyParam} completes with body {body:string}")]
fn then_key_completes(
    message_assembly_world: &mut MessageAssemblyWorld,
    key: MessageKeyParam,
    body: String,
) -> TestResult {
    message_assembly_world.assert_completed_body_for_key(key.to_key(), body.as_bytes())
}

#[then("the buffered count is {count:CountParam}")]
fn then_buffered_count(
    message_assembly_world: &mut MessageAssemblyWorld,
    count: CountParam,
) -> TestResult {
    message_assembly_world.assert_buffered_count(count.0)
}

#[rustfmt::skip]
#[then("the error is sequence mismatch expecting {expected:SequenceParam} but found {found:SequenceParam}")]
fn then_error_sequence_mismatch(
    message_assembly_world: &mut MessageAssemblyWorld,
    expected: SequenceParam,
    found: SequenceParam,
) -> TestResult {
    message_assembly_world.assert_error(MessageAssemblyErrorExpectation::SequenceMismatch {
        expected: expected.to_seq(),
        found: found.to_seq(),
    })
}

#[then(
    "the error is duplicate frame for key {key:MessageKeyParam} sequence {sequence:SequenceParam}"
)]
fn then_error_duplicate_frame(
    message_assembly_world: &mut MessageAssemblyWorld,
    key: MessageKeyParam,
    sequence: SequenceParam,
) -> TestResult {
    message_assembly_world.assert_error(MessageAssemblyErrorExpectation::DuplicateFrame {
        key: key.to_key(),
        sequence: sequence.to_seq(),
    })
}

#[then("the error is missing first frame for key {key:MessageKeyParam}")]
fn then_error_missing_first_frame(
    message_assembly_world: &mut MessageAssemblyWorld,
    key: MessageKeyParam,
) -> TestResult {
    message_assembly_world
        .assert_error(MessageAssemblyErrorExpectation::MissingFirstFrame { key: key.to_key() })
}

#[then("the error is duplicate first frame for key {key:MessageKeyParam}")]
fn then_error_duplicate_first_frame(
    message_assembly_world: &mut MessageAssemblyWorld,
    key: MessageKeyParam,
) -> TestResult {
    message_assembly_world
        .assert_error(MessageAssemblyErrorExpectation::DuplicateFirstFrame { key: key.to_key() })
}

#[then("the error is message too large for key {key:MessageKeyParam}")]
fn then_error_message_too_large(
    message_assembly_world: &mut MessageAssemblyWorld,
    key: MessageKeyParam,
) -> TestResult {
    message_assembly_world
        .assert_error(MessageAssemblyErrorExpectation::MessageTooLarge { key: key.to_key() })
}

#[then("the error is declared total too small for key {key:MessageKeyParam}")]
fn then_error_declared_total_too_small(
    message_assembly_world: &mut MessageAssemblyWorld,
    key: MessageKeyParam,
) -> TestResult {
    message_assembly_world
        .assert_error(MessageAssemblyErrorExpectation::DeclaredTotalTooSmall { key: key.to_key() })
}

#[then("the error is declared total exceeded for key {key:MessageKeyParam}")]
fn then_error_declared_total_exceeded(
    message_assembly_world: &mut MessageAssemblyWorld,
    key: MessageKeyParam,
) -> TestResult {
    message_assembly_world
        .assert_error(MessageAssemblyErrorExpectation::DeclaredTotalExceeded { key: key.to_key() })
}

#[then("the error is declared total mismatch for key {key:MessageKeyParam}")]
fn then_error_declared_total_mismatch(
    message_assembly_world: &mut MessageAssemblyWorld,
    key: MessageKeyParam,
) -> TestResult {
    message_assembly_world
        .assert_error(MessageAssemblyErrorExpectation::DeclaredTotalMismatch { key: key.to_key() })
}

#[then("key {key:MessageKeyParam} was evicted")]
fn then_key_evicted(
    message_assembly_world: &mut MessageAssemblyWorld,
    key: MessageKeyParam,
) -> TestResult {
    message_assembly_world.assert_evicted(key.to_key())
}
