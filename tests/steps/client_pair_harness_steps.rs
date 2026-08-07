//! Step definitions for client pair harness behavioural tests.

use rstest_bdd_macros::{given, then, when};

use crate::fixtures::client_pair_harness::{ClientPairHarnessWorld, TestResult};

/// Borrows the world, reporting a fixture-setup failure as a step failure.
fn world(
    client_pair_harness_world: &mut TestResult<ClientPairHarnessWorld>,
) -> TestResult<&mut ClientPairHarnessWorld> {
    client_pair_harness_world
        .as_mut()
        .map_err(|error| format!("client pair harness fixture setup failed: {error}").into())
}

#[given("a server/client pair running an echo app via the client pair harness")]
fn given_default_pair(
    client_pair_harness_world: &mut TestResult<ClientPairHarnessWorld>,
) -> TestResult {
    world(client_pair_harness_world)?.start_default_pair()
}

#[given(
    "a server/client pair with max frame length {max_frame_length:usize} via the client pair \
     harness"
)]
fn given_pair_with_frame_length(
    client_pair_harness_world: &mut TestResult<ClientPairHarnessWorld>,
    max_frame_length: usize,
) -> TestResult {
    world(client_pair_harness_world)?.start_pair_with_max_frame_length(max_frame_length)
}

#[when(
    "a request with id {id:u32} and correlation {correlation_id:u64} is sent through the client \
     pair harness"
)]
fn when_request_sent(
    client_pair_harness_world: &mut TestResult<ClientPairHarnessWorld>,
    id: u32,
    correlation_id: u64,
) -> TestResult {
    world(client_pair_harness_world)?.send_request(id, correlation_id)
}

#[then("the client pair harness response has correlation id {expected:u64} and matching payload")]
fn then_response_matches(
    client_pair_harness_world: &mut TestResult<ClientPairHarnessWorld>,
    expected: u64,
) -> TestResult {
    world(client_pair_harness_world)?.assert_response(expected)
}
