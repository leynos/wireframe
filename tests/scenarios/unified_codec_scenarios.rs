//! Scenario tests for the unified codec pipeline.

use rstest_bdd_macros::scenario;

use crate::fixtures::unified_codec::*;

#[scenario(
    path = "tests/features/unified_codec.feature",
    name = "Handler response round-trips through the unified pipeline"
)]
#[expect(
    unused_variables,
    reason = "rstest-bdd wires steps via parameters without using them directly"
)]
fn handler_response_round_trip(unified_codec_world: UnifiedCodecWorld) {}

#[scenario(
    path = "tests/features/unified_codec.feature",
    name = "Fragmented response passes through the unified pipeline"
)]
#[expect(
    unused_variables,
    reason = "rstest-bdd wires steps via parameters without using them directly"
)]
fn fragmented_response_pipeline(unified_codec_world: UnifiedCodecWorld) {}

#[scenario(
    path = "tests/features/unified_codec.feature",
    name = "Small payload passes through the pipeline unfragmented"
)]
#[expect(
    unused_variables,
    reason = "rstest-bdd wires steps via parameters without using them directly"
)]
fn small_payload_unfragmented(unified_codec_world: UnifiedCodecWorld) {}

#[scenario(
    path = "tests/features/unified_codec.feature",
    name = "Multiple sequential requests pass through the pipeline"
)]
#[expect(
    unused_variables,
    reason = "rstest-bdd wires steps via parameters without using them directly"
)]
fn multiple_sequential_requests(unified_codec_world: UnifiedCodecWorld) {}

#[scenario(
    path = "tests/features/unified_codec.feature",
    name = "Disabled fragmentation passes large payloads unchanged"
)]
#[expect(
    unused_variables,
    reason = "rstest-bdd wires steps via parameters without using them directly"
)]
fn disabled_fragmentation_large_payload(unified_codec_world: UnifiedCodecWorld) {}
