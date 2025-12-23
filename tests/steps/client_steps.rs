//! Steps for wireframe client runtime behavioural tests.

use cucumber::{given, then, when};

use crate::world::{ClientRuntimeWorld, TestResult};

#[given(expr = "a wireframe echo server allowing frames up to {int} bytes")]
async fn given_server(world: &mut ClientRuntimeWorld, max_frame_length: usize) -> TestResult {
    world.start_server(max_frame_length).await
}

#[given(expr = "a wireframe client configured with max frame length {int}")]
async fn given_client(world: &mut ClientRuntimeWorld, max_frame_length: usize) -> TestResult {
    world.connect_client(max_frame_length).await
}

#[when(expr = "the client sends a payload of {int} bytes")]
async fn when_send_payload(world: &mut ClientRuntimeWorld, size: usize) -> TestResult {
    world.send_payload(size).await
}

#[when(expr = "the client sends an oversized payload of {int} bytes")]
async fn when_send_oversized_payload(world: &mut ClientRuntimeWorld, size: usize) -> TestResult {
    world.send_payload_expect_error(size).await
}

#[then("the client receives the echoed payload")]
async fn then_receives_echo(world: &mut ClientRuntimeWorld) -> TestResult {
    world.verify_echo().await
}

#[then("the client reports a framing error")]
async fn then_reports_error(world: &mut ClientRuntimeWorld) -> TestResult {
    world.verify_error().await
}
