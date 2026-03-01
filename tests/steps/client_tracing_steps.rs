//! Step definitions for wireframe client tracing behavioural tests.

#![expect(
    clippy::unnecessary_wraps,
    reason = "step macros generate wrapper code that requires TestResult return type"
)]

use rstest_bdd_macros::{given, then, when};
use wireframe::client::TracingConfig;

use crate::fixtures::client_tracing::{ClientTracingWorld, TestResult};

// ---------------------------------------------------------------------------
// Given steps
// ---------------------------------------------------------------------------

#[given("a running echo server for tracing tests")]
fn given_echo_server(client_tracing_world: &mut ClientTracingWorld) -> TestResult {
    let rt = tokio::runtime::Runtime::new()?;
    rt.block_on(client_tracing_world.start_echo_server())
}

#[given("a client with connect timing enabled")]
fn given_client_with_connect_timing(client_tracing_world: &mut ClientTracingWorld) -> TestResult {
    client_tracing_world.set_tracing_config(TracingConfig::default().with_connect_timing(true));
    Ok(())
}

#[given("a connected tracing client with send timing enabled")]
fn given_connected_send_timing(client_tracing_world: &mut ClientTracingWorld) -> TestResult {
    let rt = tokio::runtime::Runtime::new()?;
    rt.block_on(client_tracing_world.start_echo_server())?;
    client_tracing_world.set_tracing_config(TracingConfig::default().with_send_timing(true));
    rt.block_on(client_tracing_world.connect())
}

#[given("a connected tracing client with receive timing enabled")]
fn given_connected_receive_timing(client_tracing_world: &mut ClientTracingWorld) -> TestResult {
    let rt = tokio::runtime::Runtime::new()?;
    rt.block_on(client_tracing_world.start_echo_server())?;
    client_tracing_world.set_tracing_config(TracingConfig::default().with_receive_timing(true));
    rt.block_on(client_tracing_world.connect())
}

#[given("a connected tracing client with default config")]
fn given_connected_default_config(client_tracing_world: &mut ClientTracingWorld) -> TestResult {
    let rt = tokio::runtime::Runtime::new()?;
    rt.block_on(client_tracing_world.start_echo_server())?;
    rt.block_on(client_tracing_world.connect())
}

#[given("a connected tracing client with close timing enabled")]
fn given_connected_close_timing(client_tracing_world: &mut ClientTracingWorld) -> TestResult {
    let rt = tokio::runtime::Runtime::new()?;
    rt.block_on(client_tracing_world.start_echo_server())?;
    client_tracing_world.set_tracing_config(TracingConfig::default().with_close_timing(true));
    rt.block_on(client_tracing_world.connect())
}

// ---------------------------------------------------------------------------
// When steps
// ---------------------------------------------------------------------------

#[when("the client connects to the server")]
fn when_client_connects(client_tracing_world: &mut ClientTracingWorld) -> TestResult {
    let rt = tokio::runtime::Runtime::new()?;
    rt.block_on(client_tracing_world.connect())
}

#[when("the client sends an envelope via the tracing client")]
fn when_client_sends(client_tracing_world: &mut ClientTracingWorld) -> TestResult {
    let rt = tokio::runtime::Runtime::new()?;
    rt.block_on(client_tracing_world.send_envelope())
}

#[when("the client sends and receives via the tracing client")]
fn when_client_sends_and_receives(client_tracing_world: &mut ClientTracingWorld) -> TestResult {
    let rt = tokio::runtime::Runtime::new()?;
    rt.block_on(client_tracing_world.send_and_receive())
}

#[when("the tracing client closes the connection")]
fn when_client_closes(client_tracing_world: &mut ClientTracingWorld) -> TestResult {
    let rt = tokio::runtime::Runtime::new()?;
    rt.block_on(client_tracing_world.close_connection());
    Ok(())
}

// ---------------------------------------------------------------------------
// Then steps
// ---------------------------------------------------------------------------

#[then("the tracing output contains \"client.connect\"")]
fn then_output_contains_connect(client_tracing_world: &mut ClientTracingWorld) -> TestResult {
    client_tracing_world.assert_output_contains("client.connect")
}

#[then("the tracing output contains the peer address")]
fn then_output_contains_peer_address(client_tracing_world: &mut ClientTracingWorld) -> TestResult {
    let addr = client_tracing_world.peer_addr_string();
    client_tracing_world.assert_output_contains(&addr)
}

#[then("the tracing output contains \"client.send\"")]
fn then_output_contains_send(client_tracing_world: &mut ClientTracingWorld) -> TestResult {
    client_tracing_world.assert_output_contains("client.send")
}

#[then("the tracing output contains \"frame.bytes\"")]
fn then_output_contains_frame_bytes(client_tracing_world: &mut ClientTracingWorld) -> TestResult {
    client_tracing_world.assert_output_contains("frame.bytes")
}

#[then("the tracing output contains \"client.receive\"")]
fn then_output_contains_receive(client_tracing_world: &mut ClientTracingWorld) -> TestResult {
    client_tracing_world.assert_output_contains("client.receive")
}

#[then("the tracing output contains \"elapsed_us\"")]
fn then_output_contains_elapsed_us(client_tracing_world: &mut ClientTracingWorld) -> TestResult {
    client_tracing_world.assert_output_contains("elapsed_us")
}

#[then("the tracing output does not contain \"elapsed_us\"")]
fn then_output_not_contains_elapsed_us(
    client_tracing_world: &mut ClientTracingWorld,
) -> TestResult {
    client_tracing_world.assert_output_not_contains("elapsed_us")
}

#[then("the tracing output contains \"client.close\"")]
fn then_output_contains_close(client_tracing_world: &mut ClientTracingWorld) -> TestResult {
    client_tracing_world.assert_output_contains("client.close")
}
