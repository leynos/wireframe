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

#[given("a wireframe server that replies with malformed payloads")]
fn given_malformed_server(client_runtime_world: &mut ClientRuntimeWorld) -> TestResult {
    client_runtime_world.start_malformed_response_server()
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

#[when("the client sends a payload of {size:usize} bytes expecting decode failure")]
fn when_send_payload_expect_decode_failure(
    client_runtime_world: &mut ClientRuntimeWorld,
    size: usize,
) -> TestResult {
    client_runtime_world.send_payload_expect_error(size)
}

#[when("the client sends a login request for username {username:string}")]
fn when_send_login_request(
    client_runtime_world: &mut ClientRuntimeWorld,
    username: String,
) -> TestResult {
    client_runtime_world.send_login_request(username)
}

#[then("the client receives the echoed payload")]
fn then_receives_echo(client_runtime_world: &mut ClientRuntimeWorld) -> TestResult {
    client_runtime_world.verify_echo()
}

#[then("the client reports a Wireframe transport error")]
fn then_reports_transport_error(client_runtime_world: &mut ClientRuntimeWorld) -> TestResult {
    client_runtime_world.verify_transport_wireframe_error()
}

#[then("the client reports a Wireframe decode protocol error")]
fn then_reports_decode_error(client_runtime_world: &mut ClientRuntimeWorld) -> TestResult {
    client_runtime_world.verify_decode_wireframe_error()
}

#[then("the client decodes a login acknowledgement for username {username:string}")]
fn then_decodes_login_ack(
    client_runtime_world: &mut ClientRuntimeWorld,
    username: String,
) -> TestResult {
    client_runtime_world.verify_login_acknowledgement(&username)
}
