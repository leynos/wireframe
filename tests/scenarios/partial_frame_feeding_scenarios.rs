//! Scenario tests for partial frame and fragment feeding behaviours.

use rstest_bdd_macros::scenario;

use crate::fixtures::partial_frame_feeding::*;

#[scenario(
    path = "tests/features/partial_frame_feeding.feature",
    name = "Single payload survives byte-at-a-time chunked delivery"
)]
#[expect(
    unused_variables,
    reason = "rstest-bdd wires steps via parameters without using them directly"
)]
fn single_payload_byte_chunks(partial_frame_feeding_world: PartialFrameFeedingWorld) {}

#[scenario(
    path = "tests/features/partial_frame_feeding.feature",
    name = "Multiple payloads survive misaligned chunked delivery"
)]
#[expect(
    unused_variables,
    reason = "rstest-bdd wires steps via parameters without using them directly"
)]
fn multiple_payloads_misaligned(partial_frame_feeding_world: PartialFrameFeedingWorld) {}

#[scenario(
    path = "tests/features/partial_frame_feeding.feature",
    name = "Fragmented payload is delivered as fragment frames"
)]
#[expect(
    unused_variables,
    reason = "rstest-bdd wires steps via parameters without using them directly"
)]
fn fragmented_payload_delivery(partial_frame_feeding_world: PartialFrameFeedingWorld) {}

#[scenario(
    path = "tests/features/partial_frame_feeding.feature",
    name = "Fragmented payload survives chunked delivery"
)]
#[expect(
    unused_variables,
    reason = "rstest-bdd wires steps via parameters without using them directly"
)]
fn fragmented_payload_chunked(partial_frame_feeding_world: PartialFrameFeedingWorld) {}
