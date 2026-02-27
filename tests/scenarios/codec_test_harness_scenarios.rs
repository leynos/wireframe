//! Scenario tests for codec-aware test harness behaviours.

use rstest_bdd_macros::scenario;

use crate::fixtures::codec_test_harness::*;

#[scenario(
    path = "tests/features/codec_test_harness.feature",
    name = "Payload round-trip through a custom codec driver"
)]
#[expect(
    unused_variables,
    reason = "rstest-bdd wires steps via parameters without using them directly"
)]
fn payload_round_trip(codec_test_harness_world: CodecTestHarnessWorld) {}

#[scenario(
    path = "tests/features/codec_test_harness.feature",
    name = "Frame-level driver preserves codec metadata"
)]
#[expect(
    unused_variables,
    reason = "rstest-bdd wires steps via parameters without using them directly"
)]
fn frame_level_metadata(codec_test_harness_world: CodecTestHarnessWorld) {}
