//! Steps for wireframe client lifecycle hook behavioural tests.

use cucumber::{given, then, when};

use crate::world::{ClientLifecycleWorld, EXPECTED_SETUP_STATE, TestResult};

/// Assert that a count matches the expected value, returning an appropriate error message.
fn assert_count_equals(actual: usize, expected: usize, callback_name: &str) -> TestResult {
    if actual != expected {
        return Err(format!(
            "expected {callback_name} callback to be invoked {expected} time(s), got {actual}"
        )
        .into());
    }
    Ok(())
}

/// Starts a standard echo server that keeps connections open.
#[given("a standard echo server")]
async fn given_standard_server(world: &mut ClientLifecycleWorld) -> TestResult {
    world.start_standard_server().await
}

/// Starts an echo server that disconnects immediately after accepting a connection.
#[given("a standard echo server that disconnects immediately")]
async fn given_disconnecting_server(world: &mut ClientLifecycleWorld) -> TestResult {
    world.start_disconnecting_server().await
}

/// Starts a preamble-aware server that sends an acknowledgement response.
#[given("a preamble-aware echo server that sends acknowledgement")]
async fn given_ack_server(world: &mut ClientLifecycleWorld) -> TestResult {
    world.start_ack_server().await
}

/// Connects a client with a setup callback configured.
#[when("a client connects with a setup callback")]
async fn when_connect_with_setup(world: &mut ClientLifecycleWorld) -> TestResult {
    world.connect_with_setup().await
}

/// Connects a client with both setup and teardown callbacks configured.
#[when("a client connects with setup and teardown callbacks")]
async fn when_connect_with_setup_and_teardown(world: &mut ClientLifecycleWorld) -> TestResult {
    world.connect_with_setup_and_teardown().await
}

/// Closes the client connection, triggering any teardown callbacks.
#[when("the client closes the connection")]
async fn when_client_closes(world: &mut ClientLifecycleWorld) -> TestResult {
    world.close_client().await;
    Ok(())
}

/// Connects a client with an error callback configured.
#[when("a client connects with an error callback")]
async fn when_connect_with_error_callback(world: &mut ClientLifecycleWorld) -> TestResult {
    world.connect_with_error_callback().await
}

/// Attempts to receive a message from the server, capturing any errors.
#[when("the client attempts to receive a message")]
async fn when_client_attempts_receive(world: &mut ClientLifecycleWorld) -> TestResult {
    world.attempt_receive().await
}

/// Connects a client with preamble exchange and lifecycle callbacks configured.
#[when("a client connects with preamble and lifecycle callbacks")]
async fn when_connect_with_preamble_and_lifecycle(world: &mut ClientLifecycleWorld) -> TestResult {
    world.connect_with_preamble_and_lifecycle().await
}

/// Asserts that the setup callback was invoked exactly once.
#[then("the setup callback is invoked exactly once")]
fn then_setup_invoked_once(world: &mut ClientLifecycleWorld) -> TestResult {
    assert_count_equals(world.setup_count(), 1, "setup")
}

/// Asserts that the teardown callback was invoked exactly once.
#[then("the teardown callback is invoked exactly once")]
fn then_teardown_invoked_once(world: &mut ClientLifecycleWorld) -> TestResult {
    assert_count_equals(world.teardown_count(), 1, "teardown")
}

/// Asserts that the teardown callback received the expected state from setup.
#[then("the teardown callback receives the state from setup")]
fn then_teardown_receives_state(world: &mut ClientLifecycleWorld) -> TestResult {
    let state = world.teardown_received_state();
    let expected = EXPECTED_SETUP_STATE as usize;
    if state != expected {
        return Err(format!("expected teardown to receive state {expected}, got {state}").into());
    }
    Ok(())
}

/// Asserts that the error callback was invoked at least once.
#[then("the error callback is invoked")]
fn then_error_callback_invoked(world: &mut ClientLifecycleWorld) -> TestResult {
    let count = world.error_count();
    if count == 0 {
        return Err("expected error callback to be invoked at least once".into());
    }
    Ok(())
}

/// Asserts that the preamble success callback was invoked.
#[then("the preamble success callback is invoked")]
fn then_preamble_success_invoked(world: &mut ClientLifecycleWorld) -> TestResult {
    if !world.preamble_success_invoked() {
        return Err("expected preamble success callback to be invoked".into());
    }
    Ok(())
}

/// Asserts that the captured client error is a Disconnected error.
#[then("the client error is Disconnected")]
fn then_client_error_is_disconnected(world: &mut ClientLifecycleWorld) -> TestResult {
    let last_error = world
        .last_error()
        .ok_or("expected a captured client error in world.last_error")?;

    match last_error {
        wireframe::ClientError::Disconnected => Ok(()),
        other => Err(format!("expected ClientError::Disconnected, got {other:?}").into()),
    }
}
