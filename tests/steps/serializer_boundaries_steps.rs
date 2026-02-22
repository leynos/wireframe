//! Step definitions for serializer boundary behavioural tests.

use rstest_bdd_macros::{given, then, when};

use crate::fixtures::serializer_boundaries::{SerializerBoundariesWorld, TestResult};

#[given("a legacy payload value {value:u32}")]
fn given_legacy_payload_value(
    serializer_boundaries_world: &mut SerializerBoundariesWorld,
    value: u32,
) {
    serializer_boundaries_world.set_legacy_value(value);
}

#[when("the legacy payload is encoded and decoded")]
fn when_legacy_payload_round_trip(
    serializer_boundaries_world: &mut SerializerBoundariesWorld,
) -> TestResult {
    serializer_boundaries_world.round_trip_legacy_payload()
}

#[then("the decoded legacy payload value is {expected:u32}")]
fn then_decoded_legacy_value(
    serializer_boundaries_world: &mut SerializerBoundariesWorld,
    expected: u32,
) -> TestResult {
    serializer_boundaries_world.assert_decoded_legacy_value(expected)
}

#[given("deserialize context message id {message_id:u32} and correlation id {correlation_id:u64}")]
fn given_deserialize_context(
    serializer_boundaries_world: &mut SerializerBoundariesWorld,
    message_id: u32,
    correlation_id: u64,
) {
    serializer_boundaries_world.set_deserialize_context(message_id, correlation_id);
}

#[when("a context-aware serializer decodes with context")]
fn when_context_aware_decode(
    serializer_boundaries_world: &mut SerializerBoundariesWorld,
) -> TestResult {
    serializer_boundaries_world.decode_with_context()
}

#[then("the captured message id is {expected:u32}")]
fn then_captured_message_id(
    serializer_boundaries_world: &mut SerializerBoundariesWorld,
    expected: u32,
) -> TestResult {
    serializer_boundaries_world.assert_captured_message_id(expected)
}

#[then("the captured correlation id is {expected:u64}")]
fn then_captured_correlation_id(
    serializer_boundaries_world: &mut SerializerBoundariesWorld,
    expected: u64,
) -> TestResult {
    serializer_boundaries_world.assert_captured_correlation_id(expected)
}
