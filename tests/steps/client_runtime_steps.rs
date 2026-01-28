//! Step definitions for wireframe client runtime behavioural tests.

use rstest_bdd_macros::{given, then, when};

use crate::fixtures::client_runtime::{ClientRuntimeWorld, TestResult};

#[given("a wireframe echo server allowing frames up to {max_frame_length:usize} bytes")]
fn given_server(
    client_runtime_world: &mut ClientRuntimeWorld,
    max_frame_length: usize,
) -> TestResult {
    client_runtime_world.start_server(max_frame_length)
}

#[given("a wireframe client configured with max frame length {max_frame_length:usize}")]
fn given_client(
    client_runtime_world: &mut ClientRuntimeWorld,
    max_frame_length: usize,
) -> TestResult {
    client_runtime_world.connect_client(max_frame_length)
}

#[when("the client sends a payload of {size:usize} bytes")]
fn when_send_payload(client_runtime_world: &mut ClientRuntimeWorld, size: usize) -> TestResult {
    client_runtime_world.send_payload(size)
}

#[when("the client sends an oversized payload of {size:usize} bytes")]
fn when_send_oversized_payload(
    client_runtime_world: &mut ClientRuntimeWorld,
    size: usize,
) -> TestResult {
    client_runtime_world.send_payload_expect_error(size)
}

#[then("the client receives the echoed payload")]
fn then_receives_echo(client_runtime_world: &mut ClientRuntimeWorld) -> TestResult {
    client_runtime_world.verify_echo()
}

#[then("the client reports a framing error")]
fn then_reports_error(client_runtime_world: &mut ClientRuntimeWorld) -> TestResult {
    client_runtime_world.verify_error()
}
