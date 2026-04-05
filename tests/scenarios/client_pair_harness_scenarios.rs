//! Scenario tests for in-process server and client pair harness behaviour.

use rstest_bdd_macros::scenario;

use crate::fixtures::client_pair_harness::*;

#[scenario(
    path = "tests/features/client_pair_harness.feature",
    name = "Downstream crate verifies a request/response contract through the pair harness"
)]
fn client_pair_harness_round_trip(client_pair_harness_world: ClientPairHarnessWorld) {
    let _ = &client_pair_harness_world;
}

#[scenario(
    path = "tests/features/client_pair_harness.feature",
    name = "Downstream crate supplies non-default client configuration through the pair harness"
)]
fn client_pair_harness_custom_config(client_pair_harness_world: ClientPairHarnessWorld) {
    let _ = &client_pair_harness_world;
}
