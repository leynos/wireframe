//! Scenario tests for codec property round-trip behaviours.

use rstest_bdd_macros::scenario;

use crate::fixtures::codec_property_roundtrip::*;

#[scenario(
    path = "tests/features/codec_property_roundtrip.feature",
    name = "Generated default codec payload sequences round-trip"
)]
#[expect(
    unused_variables,
    reason = "rstest-bdd wires steps via parameters without using them directly"
)]
fn generated_default_sequences(codec_property_roundtrip_world: CodecPropertyRoundtripWorld) {}

#[scenario(
    path = "tests/features/codec_property_roundtrip.feature",
    name = "Generated malformed default codec frames are rejected"
)]
#[expect(
    unused_variables,
    reason = "rstest-bdd wires steps via parameters without using them directly"
)]
fn generated_default_malformed(codec_property_roundtrip_world: CodecPropertyRoundtripWorld) {}

#[scenario(
    path = "tests/features/codec_property_roundtrip.feature",
    name = "Generated mock codec sequences keep state deterministic"
)]
#[expect(
    unused_variables,
    reason = "rstest-bdd wires steps via parameters without using them directly"
)]
fn generated_mock_sequences(codec_property_roundtrip_world: CodecPropertyRoundtripWorld) {}
