//! Step definitions for client request hook behavioural tests.

use rstest_bdd_macros::{given, then, when};

use crate::fixtures::client_request_hooks::{ClientRequestHooksWorld, TestResult};

#[given("an envelope echo server for hook testing")]
fn given_echo_server(client_request_hooks_world: &mut ClientRequestHooksWorld) -> TestResult {
    let rt = tokio::runtime::Runtime::new()?;
    rt.block_on(client_request_hooks_world.start_echo_server())
}

#[given("a client with a before_send counter hook")]
fn given_before_send_counter(
    client_request_hooks_world: &mut ClientRequestHooksWorld,
) -> TestResult {
    let rt = tokio::runtime::Runtime::new()?;
    rt.block_on(client_request_hooks_world.connect_with_before_send_counter())
}

#[given("a client with an after_receive counter hook")]
fn given_after_receive_counter(
    client_request_hooks_world: &mut ClientRequestHooksWorld,
) -> TestResult {
    let rt = tokio::runtime::Runtime::new()?;
    rt.block_on(client_request_hooks_world.connect_with_after_receive_counter())
}

#[given("a client with two before_send hooks that append markers")]
fn given_marker_hooks(client_request_hooks_world: &mut ClientRequestHooksWorld) -> TestResult {
    let rt = tokio::runtime::Runtime::new()?;
    rt.block_on(client_request_hooks_world.connect_with_marker_hooks())
}

#[given("a client with both counter hooks")]
fn given_both_counters(client_request_hooks_world: &mut ClientRequestHooksWorld) -> TestResult {
    let rt = tokio::runtime::Runtime::new()?;
    rt.block_on(client_request_hooks_world.connect_with_both_counters())
}

#[when("the client sends an envelope via the hooked client")]
fn when_send_envelope(client_request_hooks_world: &mut ClientRequestHooksWorld) -> TestResult {
    let rt = tokio::runtime::Runtime::new()?;
    rt.block_on(client_request_hooks_world.send_envelope())
}

#[when("the client sends and receives an envelope via the hooked client")]
fn when_send_and_receive(client_request_hooks_world: &mut ClientRequestHooksWorld) -> TestResult {
    let rt = tokio::runtime::Runtime::new()?;
    rt.block_on(client_request_hooks_world.send_and_receive_envelope())
}

#[when("the client performs a correlated call via the hooked client")]
fn when_correlated_call(client_request_hooks_world: &mut ClientRequestHooksWorld) -> TestResult {
    let rt = tokio::runtime::Runtime::new()?;
    rt.block_on(client_request_hooks_world.perform_correlated_call())
}

#[then("the before_send counter is {expected:usize}")]
fn then_before_send_counter(
    client_request_hooks_world: &mut ClientRequestHooksWorld,
    expected: usize,
) -> TestResult {
    let actual = client_request_hooks_world.before_send_count();
    if actual != expected {
        return Err(format!("expected before_send counter to be {expected}, got {actual}").into());
    }
    Ok(())
}

#[then("the after_receive counter is {expected:usize}")]
fn then_after_receive_counter(
    client_request_hooks_world: &mut ClientRequestHooksWorld,
    expected: usize,
) -> TestResult {
    let actual = client_request_hooks_world.after_receive_count();
    if actual != expected {
        return Err(
            format!("expected after_receive counter to be {expected}, got {actual}").into(),
        );
    }
    Ok(())
}

#[then("the markers appear in registration order")]
fn then_markers_in_order(client_request_hooks_world: &mut ClientRequestHooksWorld) -> TestResult {
    let log = client_request_hooks_world.marker_log();
    if log != [b'A', b'B'] {
        return Err(format!("expected markers [A, B] in registration order, got {log:?}").into());
    }
    Ok(())
}
