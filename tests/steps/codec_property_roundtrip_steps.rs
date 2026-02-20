//! Step definitions for codec property round-trip behavioural tests.

use rstest_bdd_macros::{given, then, when};

use crate::fixtures::codec_property_roundtrip::{CodecPropertyRoundtripWorld, TestResult};

#[given("generated codec checks run with {cases:usize} cases")]
fn given_generated_cases(
    codec_property_roundtrip_world: &mut CodecPropertyRoundtripWorld,
    cases: usize,
) -> TestResult {
    codec_property_roundtrip_world.configure_cases(cases)
}

#[when("generated boundary payload sequences are exercised for the default codec")]
fn when_default_sequences(
    codec_property_roundtrip_world: &mut CodecPropertyRoundtripWorld,
) -> TestResult {
    codec_property_roundtrip_world.run_default_sequence_checks()
}

#[when("generated malformed frame checks are exercised for the default codec")]
fn when_default_malformed(
    codec_property_roundtrip_world: &mut CodecPropertyRoundtripWorld,
) -> TestResult {
    codec_property_roundtrip_world.run_default_malformed_checks()
}

#[when("generated stateful sequence checks are exercised for the mock codec")]
fn when_mock_sequences(
    codec_property_roundtrip_world: &mut CodecPropertyRoundtripWorld,
) -> TestResult {
    codec_property_roundtrip_world.run_mock_sequence_checks()
}

#[then("the default codec generated sequence checks succeed")]
fn then_default_sequences(
    codec_property_roundtrip_world: &mut CodecPropertyRoundtripWorld,
) -> TestResult {
    codec_property_roundtrip_world.assert_default_sequence_checks_passed()
}

#[then("the default codec generated malformed checks succeed")]
fn then_default_malformed(
    codec_property_roundtrip_world: &mut CodecPropertyRoundtripWorld,
) -> TestResult {
    codec_property_roundtrip_world.assert_default_malformed_checks_passed()
}

#[then("the mock codec generated sequence checks succeed")]
fn then_mock_sequences(
    codec_property_roundtrip_world: &mut CodecPropertyRoundtripWorld,
) -> TestResult {
    codec_property_roundtrip_world.assert_mock_sequence_checks_passed()
}
