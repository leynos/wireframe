//! Step definitions for test observability behavioural tests.

use rstest_bdd_macros::{given, then, when};

use crate::fixtures::test_observability::{TestObservabilityWorld, TestResult};

#[given("an observability harness is acquired")]
fn given_observability_harness(
    test_observability_world: &mut TestObservabilityWorld,
) -> TestResult {
    test_observability_world.acquire_harness()
}

#[when("a codec error metric is recorded")]
fn when_codec_error_recorded(test_observability_world: &mut TestObservabilityWorld) -> TestResult {
    test_observability_world.record_codec_error()
}

#[when("a warning log is emitted")]
fn when_warning_log_emitted(test_observability_world: &mut TestObservabilityWorld) -> TestResult {
    test_observability_world.emit_warning_log()
}

#[when("the observability state is cleared")]
fn when_state_cleared(test_observability_world: &mut TestObservabilityWorld) -> TestResult {
    test_observability_world.clear_state()
}

#[then("the codec error counter equals {expected:u64}")]
fn then_codec_error_counter_equals(
    test_observability_world: &mut TestObservabilityWorld,
    expected: u64,
) -> TestResult {
    test_observability_world.verify_codec_error_counter(expected)
}

#[then("the log buffer contains the expected message")]
fn then_log_contains_message(test_observability_world: &mut TestObservabilityWorld) -> TestResult {
    test_observability_world.verify_log_contains()
}
