//! Scenario tests for stateful codec behaviours.

use rstest_bdd_macros::scenario;

use crate::fixtures::codec_stateful::*;

#[scenario(
    path = "tests/features/codec_stateful.feature",
    name = "Sequence counters reset per connection"
)]
#[expect(
    unused_variables,
    reason = "rstest-bdd wires steps via parameters without using them directly"
)]
fn sequence_counters_reset(codec_stateful_world: CodecStatefulWorld) {}
