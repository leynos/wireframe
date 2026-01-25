//! Scenario tests for stateful codec behaviours.

use rstest_bdd_macros::scenario;

use crate::fixtures::codec_stateful::*;

#[scenario(
    path = "tests/features/codec_stateful.feature",
    name = "Sequence counters reset per connection"
)]
fn sequence_counters_reset(codec_stateful_world: CodecStatefulWorld) {
    let _ = codec_stateful_world;
}
