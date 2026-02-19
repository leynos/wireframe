//! Step definitions for wireframe client lifecycle hook behavioural tests.

use rstest_bdd_macros::{given, then, when};

use crate::fixtures::client_lifecycle::{ClientLifecycleWorld, EXPECTED_SETUP_STATE, TestResult};

fn assert_count_equals(actual: usize, expected: usize, callback_name: &str) -> TestResult {
    if actual != expected {
        return Err(format!(
            "expected {callback_name} callback to be invoked {expected} time(s), got {actual}"
        )
        .into());
    }
    Ok(())
}

#[given("a standard echo server")]
fn given_standard_server(client_lifecycle_world: &mut ClientLifecycleWorld) -> TestResult {
    let rt = tokio::runtime::Runtime::new()?;
    rt.block_on(client_lifecycle_world.start_standard_server())
}

#[given("a standard echo server that disconnects immediately")]
fn given_disconnecting_server(client_lifecycle_world: &mut ClientLifecycleWorld) -> TestResult {
    let rt = tokio::runtime::Runtime::new()?;
    rt.block_on(client_lifecycle_world.start_disconnecting_server())
}

#[given("a preamble-aware echo server that sends acknowledgement")]
fn given_ack_server(client_lifecycle_world: &mut ClientLifecycleWorld) -> TestResult {
    let rt = tokio::runtime::Runtime::new()?;
    rt.block_on(client_lifecycle_world.start_ack_server())
}

#[when("a client connects with a setup callback")]
fn when_connect_with_setup(client_lifecycle_world: &mut ClientLifecycleWorld) -> TestResult {
    let rt = tokio::runtime::Runtime::new()?;
    rt.block_on(client_lifecycle_world.connect_with_setup())
}

#[when("a client connects with setup and teardown callbacks")]
fn when_connect_with_setup_and_teardown(
    client_lifecycle_world: &mut ClientLifecycleWorld,
) -> TestResult {
    let rt = tokio::runtime::Runtime::new()?;
    rt.block_on(client_lifecycle_world.connect_with_setup_and_teardown())
}

#[when("the client closes the connection")]
fn when_client_closes(client_lifecycle_world: &mut ClientLifecycleWorld) -> TestResult {
    let rt = tokio::runtime::Runtime::new()?;
    rt.block_on(client_lifecycle_world.close_client());
    Ok(())
}

#[when("a client connects with an error callback")]
fn when_connect_with_error_callback(
    client_lifecycle_world: &mut ClientLifecycleWorld,
) -> TestResult {
    let rt = tokio::runtime::Runtime::new()?;
    rt.block_on(client_lifecycle_world.connect_with_error_callback())
}

#[when("the client attempts to receive a message")]
fn when_client_attempts_receive(client_lifecycle_world: &mut ClientLifecycleWorld) -> TestResult {
    let rt = tokio::runtime::Runtime::new()?;
    rt.block_on(client_lifecycle_world.attempt_receive())
}

#[when("a client connects with preamble and lifecycle callbacks")]
fn when_connect_with_preamble_and_lifecycle(
    client_lifecycle_world: &mut ClientLifecycleWorld,
) -> TestResult {
    let rt = tokio::runtime::Runtime::new()?;
    rt.block_on(client_lifecycle_world.connect_with_preamble_and_lifecycle())
}

#[then("the setup callback is invoked exactly once")]
fn then_setup_invoked_once(client_lifecycle_world: &mut ClientLifecycleWorld) -> TestResult {
    assert_count_equals(client_lifecycle_world.setup_count(), 1, "setup")
}

#[then("the teardown callback is invoked exactly once")]
fn then_teardown_invoked_once(client_lifecycle_world: &mut ClientLifecycleWorld) -> TestResult {
    assert_count_equals(client_lifecycle_world.teardown_count(), 1, "teardown")
}

#[then("the teardown callback receives the state from setup")]
fn then_teardown_receives_state(client_lifecycle_world: &mut ClientLifecycleWorld) -> TestResult {
    let state = client_lifecycle_world.teardown_received_state();
    let expected = EXPECTED_SETUP_STATE as usize;
    if state != expected {
        return Err(format!("expected teardown to receive state {expected}, got {state}").into());
    }
    Ok(())
}

#[then("the error callback is invoked")]
fn then_error_callback_invoked(client_lifecycle_world: &mut ClientLifecycleWorld) -> TestResult {
    let count = client_lifecycle_world.error_count();
    if count == 0 {
        return Err("expected error callback to be invoked at least once".into());
    }
    Ok(())
}

#[then("the preamble success callback is invoked")]
fn then_preamble_success_invoked(client_lifecycle_world: &mut ClientLifecycleWorld) -> TestResult {
    if !client_lifecycle_world.preamble_success_invoked() {
        return Err("expected preamble success callback to be invoked".into());
    }
    Ok(())
}

#[then("the client error is a Wireframe transport error")]
fn then_client_error_is_wireframe_transport_error(
    client_lifecycle_world: &mut ClientLifecycleWorld,
) -> TestResult {
    let last_error = client_lifecycle_world
        .last_error()
        .ok_or("expected a captured client error in world.last_error")?;

    match last_error {
        wireframe::client::ClientError::Wireframe(wireframe::WireframeError::Io(_)) => Ok(()),
        other => Err(format!(
            "expected ClientError::Wireframe(WireframeError::Io(_)), got {other:?}"
        )
        .into()),
    }
}
