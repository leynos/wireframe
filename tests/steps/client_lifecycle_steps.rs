//! Steps for wireframe client lifecycle hook behavioural tests.

use cucumber::{given, then, when};

use crate::world::{ClientLifecycleWorld, TestResult};

#[given("a standard echo server")]
async fn given_standard_server(world: &mut ClientLifecycleWorld) -> TestResult {
    world.start_standard_server().await
}

#[given("a standard echo server that disconnects immediately")]
async fn given_disconnecting_server(world: &mut ClientLifecycleWorld) -> TestResult {
    world.start_disconnecting_server().await
}

#[given("a preamble-aware echo server that sends acknowledgement")]
async fn given_ack_server(world: &mut ClientLifecycleWorld) -> TestResult {
    world.start_ack_server().await
}

#[when("a client connects with a setup callback")]
async fn when_connect_with_setup(world: &mut ClientLifecycleWorld) -> TestResult {
    world.connect_with_setup().await
}

#[when("a client connects with setup and teardown callbacks")]
async fn when_connect_with_setup_and_teardown(world: &mut ClientLifecycleWorld) -> TestResult {
    world.connect_with_setup_and_teardown().await
}

#[when("the client closes the connection")]
async fn when_client_closes(world: &mut ClientLifecycleWorld) { world.close_client().await; }

#[when("a client connects with an error callback")]
async fn when_connect_with_error_callback(world: &mut ClientLifecycleWorld) -> TestResult {
    world.connect_with_error_callback().await
}

#[when("the client attempts to receive a message")]
async fn when_client_attempts_receive(world: &mut ClientLifecycleWorld) -> TestResult {
    world.attempt_receive().await
}

#[when("a client connects with preamble and lifecycle callbacks")]
async fn when_connect_with_preamble_and_lifecycle(world: &mut ClientLifecycleWorld) -> TestResult {
    world.connect_with_preamble_and_lifecycle().await
}

#[then("the setup callback is invoked exactly once")]
fn then_setup_invoked_once(world: &mut ClientLifecycleWorld) -> TestResult {
    let count = world.setup_count();
    if count != 1 {
        return Err(format!("expected setup callback to be invoked once, got {count}").into());
    }
    world.abort_server();
    Ok(())
}

#[then("the teardown callback is invoked exactly once")]
fn then_teardown_invoked_once(world: &mut ClientLifecycleWorld) -> TestResult {
    let count = world.teardown_count();
    if count != 1 {
        return Err(format!("expected teardown callback to be invoked once, got {count}").into());
    }
    Ok(())
}

#[then("the teardown callback receives the state from setup")]
fn then_teardown_receives_state(world: &mut ClientLifecycleWorld) -> TestResult {
    let state = world.teardown_received_state();
    if state != 42 {
        return Err(format!("expected teardown to receive state 42, got {state}").into());
    }
    world.abort_server();
    Ok(())
}

#[then("the error callback is invoked")]
fn then_error_callback_invoked(world: &mut ClientLifecycleWorld) -> TestResult {
    let count = world.error_count();
    if count == 0 {
        return Err("expected error callback to be invoked at least once".into());
    }
    world.abort_server();
    Ok(())
}

#[then("the preamble success callback is invoked")]
fn then_preamble_success_invoked(world: &mut ClientLifecycleWorld) -> TestResult {
    if !world.preamble_success_invoked() {
        return Err("expected preamble success callback to be invoked".into());
    }
    Ok(())
}

#[then("the setup callback is invoked after preamble exchange")]
fn then_setup_after_preamble(world: &mut ClientLifecycleWorld) -> TestResult {
    let count = world.setup_count();
    if count != 1 {
        return Err(format!("expected setup callback to be invoked once, got {count}").into());
    }
    world.abort_server();
    Ok(())
}
