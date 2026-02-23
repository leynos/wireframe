//! Scenario tests for serializer boundary feature.

use rstest_bdd_macros::scenario;

use crate::fixtures::serializer_boundaries::*;

#[scenario(
    path = "tests/features/serializer_boundaries.feature",
    name = "Legacy message round-trips through serializer-agnostic adapters"
)]
fn legacy_message_round_trip(serializer_boundaries_world: SerializerBoundariesWorld) {
    let _ = serializer_boundaries_world;
}

#[scenario(
    path = "tests/features/serializer_boundaries.feature",
    name = "Metadata context is forwarded to deserialization"
)]
fn metadata_context_forwarded(serializer_boundaries_world: SerializerBoundariesWorld) {
    let _ = serializer_boundaries_world;
}
