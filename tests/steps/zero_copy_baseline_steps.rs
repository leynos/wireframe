//! Step definitions for the zero-copy baseline gate.

use rstest_bdd_macros::{given, then, when};

use crate::fixtures::zero_copy_baseline::{TestResult, ZeroCopyBaselineWorld};

#[given("a default-path envelope carrying a large payload")]
fn given_large_envelope(zero_copy_baseline_world: &mut ZeroCopyBaselineWorld) {
    zero_copy_baseline_world.arrive_with_large_payload();
}

#[when("the envelope passes through a read-only middleware chain")]
fn when_read_only_chain(zero_copy_baseline_world: &mut ZeroCopyBaselineWorld) {
    zero_copy_baseline_world.pass_through_read_only_middleware();
}

#[then("the payload buffer is the one the envelope arrived with")]
fn then_buffer_preserved(zero_copy_baseline_world: &mut ZeroCopyBaselineWorld) -> TestResult {
    zero_copy_baseline_world.assert_payload_buffer_preserved()
}

#[then("the read-only chain leaves the payload unchanged")]
fn then_payload_unchanged(zero_copy_baseline_world: &mut ZeroCopyBaselineWorld) -> TestResult {
    zero_copy_baseline_world.assert_payload_unchanged()
}

#[when("the envelope passes through a middleware that copies its frame")]
fn when_copying_chain(zero_copy_baseline_world: &mut ZeroCopyBaselineWorld) {
    zero_copy_baseline_world.pass_through_copying_middleware();
}

#[then("the payload buffer differs from the one the envelope arrived with")]
fn then_buffer_relocated(zero_copy_baseline_world: &mut ZeroCopyBaselineWorld) -> TestResult {
    zero_copy_baseline_world.assert_payload_buffer_relocated()
}

#[given("a framed default-path message carrying a large payload")]
fn given_framed_message(zero_copy_baseline_world: &mut ZeroCopyBaselineWorld) {
    zero_copy_baseline_world.arrive_with_large_payload();
}

#[when("the message is decoded into an envelope")]
fn when_message_decoded(zero_copy_baseline_world: &mut ZeroCopyBaselineWorld) -> TestResult {
    zero_copy_baseline_world.decode_framed_message()
}

#[then("the deserialized copied-byte count equals the payload length")]
fn then_deserialized_equals_payload(
    zero_copy_baseline_world: &mut ZeroCopyBaselineWorld,
) -> TestResult {
    zero_copy_baseline_world.assert_deserialized_equals_payload()
}
