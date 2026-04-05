//! Step definitions for client pair harness behavioural tests.

use rstest_bdd_macros::{given, then, when};

use crate::fixtures::client_pair_harness::{ClientPairHarnessWorld, TestResult};

#[given("a server/client pair running an echo app via the client pair harness")]
fn given_default_pair(client_pair_harness_world: &mut ClientPairHarnessWorld) -> TestResult {
    client_pair_harness_world.start_default_pair()
}

#[given(
    "a server/client pair with max frame length {max_frame_length:usize} via the client pair \
     harness"
)]
fn given_pair_with_frame_length(
    client_pair_harness_world: &mut ClientPairHarnessWorld,
    max_frame_length: usize,
) -> TestResult {
    client_pair_harness_world.start_pair_with_max_frame_length(max_frame_length)
}

#[when(
    "a request with id {id:u32} and correlation {correlation_id:u64} is sent through the client \
     pair harness"
)]
fn when_request_sent(
    client_pair_harness_world: &mut ClientPairHarnessWorld,
    id: u32,
    correlation_id: u64,
) -> TestResult {
    client_pair_harness_world.send_request(id, correlation_id)
}

#[then("the client pair harness response has correlation id {expected:u64} and matching payload")]
fn then_response_matches(
    client_pair_harness_world: &mut ClientPairHarnessWorld,
    expected: u64,
) -> TestResult {
    client_pair_harness_world.assert_response(expected)
}
