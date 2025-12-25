//! Steps for wireframe client preamble behavioural tests.

use cucumber::{given, then, when};

use crate::world::{ClientPreambleWorld, TestResult};

#[given("a preamble-aware echo server")]
async fn given_preamble_server(world: &mut ClientPreambleWorld) -> TestResult {
    world.start_preamble_server().await
}

#[given("a preamble-aware echo server that sends acknowledgement")]
async fn given_ack_server(world: &mut ClientPreambleWorld) -> TestResult {
    world.start_ack_server().await
}

#[given("a slow preamble server that never responds")]
async fn given_slow_server(world: &mut ClientPreambleWorld) -> TestResult {
    world.start_slow_server().await
}

#[given("a standard echo server")]
async fn given_standard_server(world: &mut ClientPreambleWorld) -> TestResult {
    world.start_standard_server().await
}

#[when(expr = "a client connects with a preamble containing version {int}")]
async fn when_connect_with_version(world: &mut ClientPreambleWorld, version: u16) -> TestResult {
    world.connect_with_preamble(version).await
}

#[when("a client connects with a preamble and reads the acknowledgement")]
async fn when_connect_with_ack(world: &mut ClientPreambleWorld) -> TestResult {
    world.connect_with_ack().await
}

#[when(expr = "a client connects with a {int}ms preamble timeout")]
async fn when_connect_with_timeout(world: &mut ClientPreambleWorld, timeout_ms: u64) -> TestResult {
    world.connect_with_timeout(timeout_ms).await
}

#[when("a client connects without a preamble")]
async fn when_connect_without_preamble(world: &mut ClientPreambleWorld) -> TestResult {
    world.connect_without_preamble().await
}

#[then(expr = "the server receives the preamble with version {int}")]
fn then_server_receives_preamble(world: &mut ClientPreambleWorld, version: u16) -> TestResult {
    if world.server_received_version() != Some(version) {
        return Err(format!(
            "expected server to receive version {}, got {:?}",
            version,
            world.server_received_version()
        )
        .into());
    }
    Ok(())
}

#[then("the client success callback is invoked")]
fn then_success_callback_invoked(world: &mut ClientPreambleWorld) -> TestResult {
    if !world.success_invoked() {
        return Err("expected success callback to be invoked".into());
    }
    world.await_server();
    Ok(())
}

#[then("the client receives an accepted acknowledgement")]
fn then_receives_ack(world: &mut ClientPreambleWorld) -> TestResult {
    if !world.ack_accepted() {
        return Err("expected client to receive accepted acknowledgement".into());
    }
    world.await_server();
    Ok(())
}

#[then("the client fails with a timeout error")]
fn then_timeout_error(world: &mut ClientPreambleWorld) -> TestResult {
    if !world.was_timeout_error() {
        return Err("expected timeout error".into());
    }
    Ok(())
}

#[then("the failure callback is invoked")]
fn then_failure_callback_invoked(world: &mut ClientPreambleWorld) -> TestResult {
    if !world.failure_invoked() {
        return Err("expected failure callback to be invoked".into());
    }
    world.await_server();
    Ok(())
}

#[then("the client connects successfully")]
fn then_connects_successfully(world: &mut ClientPreambleWorld) -> TestResult {
    if !world.is_connected() {
        return Err("expected client to connect successfully".into());
    }
    world.await_server();
    Ok(())
}
