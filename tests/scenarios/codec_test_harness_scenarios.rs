//! Scenario tests for codec-aware test harness behaviours.

use rstest_bdd_macros::scenario;

use crate::fixtures::codec_test_harness::*;

#[scenario(
    path = "tests/features/codec_test_harness.feature",
    name = "Payload round-trip through a custom codec driver"
)]
fn payload_round_trip(codec_test_harness_world: CodecTestHarnessWorld) {
    let _ = codec_test_harness_world;
}

#[scenario(
    path = "tests/features/codec_test_harness.feature",
    name = "Frame-level driver preserves codec metadata"
)]
fn frame_level_metadata(codec_test_harness_world: CodecTestHarnessWorld) {
    let _ = codec_test_harness_world;
}
