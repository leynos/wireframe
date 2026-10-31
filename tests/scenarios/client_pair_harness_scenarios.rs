//! Scenario tests for in-process server and client pair harness behaviour.

use rstest_bdd_macros::scenario;
use wireframe_testing::TestError;

use crate::fixtures::client_pair_harness::{ClientPairHarnessWorld, client_pair_harness_world};

#[scenario(
    path = "tests/features/client_pair_harness.feature",
    name = "Downstream crate verifies a request/response contract through the pair harness"
)]
fn client_pair_harness_round_trip(
    client_pair_harness_world: Result<ClientPairHarnessWorld, TestError>,
) -> Result<(), TestError> {
    Ok(())
}

#[scenario(
    path = "tests/features/client_pair_harness.feature",
    name = "Downstream crate supplies non-default client configuration through the pair harness"
)]
fn client_pair_harness_custom_config(
    client_pair_harness_world: Result<ClientPairHarnessWorld, TestError>,
) -> Result<(), TestError> {
    Ok(())
}
