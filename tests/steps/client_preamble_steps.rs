//! Step definitions for wireframe client preamble behavioural tests.

use rstest_bdd_macros::{given, then, when};

use crate::fixtures::client_preamble::{ClientPreambleWorld, TestResult};

#[given("a preamble-aware echo server")]
fn given_preamble_server(client_preamble_world: &mut ClientPreambleWorld) -> TestResult {
    let rt = tokio::runtime::Runtime::new()?;
    rt.block_on(client_preamble_world.start_preamble_server())
}

#[given("a preamble-aware echo server that sends an acknowledgement preamble")]
fn given_ack_server(client_preamble_world: &mut ClientPreambleWorld) -> TestResult {
    let rt = tokio::runtime::Runtime::new()?;
    rt.block_on(client_preamble_world.start_ack_server())
}

#[given("a slow preamble server that never responds")]
fn given_slow_server(client_preamble_world: &mut ClientPreambleWorld) -> TestResult {
    let rt = tokio::runtime::Runtime::new()?;
    rt.block_on(client_preamble_world.start_slow_server())
}

#[given("a standard echo server without preamble support")]
fn given_standard_server(client_preamble_world: &mut ClientPreambleWorld) -> TestResult {
    let rt = tokio::runtime::Runtime::new()?;
    rt.block_on(client_preamble_world.start_standard_server())
}

#[when("a client connects with a preamble containing version {version:u16}")]
fn when_connect_with_version(
    client_preamble_world: &mut ClientPreambleWorld,
    version: u16,
) -> TestResult {
    let rt = tokio::runtime::Runtime::new()?;
    rt.block_on(client_preamble_world.connect_with_preamble(version))
}

#[when("a client connects with a preamble and reads the acknowledgement")]
fn when_connect_with_ack(client_preamble_world: &mut ClientPreambleWorld) -> TestResult {
    let rt = tokio::runtime::Runtime::new()?;
    rt.block_on(client_preamble_world.connect_with_ack())
}

#[when("a client connects with a {timeout_ms:u64}ms preamble timeout")]
fn when_connect_with_timeout(
    client_preamble_world: &mut ClientPreambleWorld,
    timeout_ms: u64,
) -> TestResult {
    let rt = tokio::runtime::Runtime::new()?;
    rt.block_on(client_preamble_world.connect_with_timeout(timeout_ms))
}

#[when("a client connects without a preamble")]
fn when_connect_without_preamble(client_preamble_world: &mut ClientPreambleWorld) -> TestResult {
    let rt = tokio::runtime::Runtime::new()?;
    rt.block_on(client_preamble_world.connect_without_preamble())
}

#[then("the server receives the preamble with version {version:u16}")]
fn then_server_receives_preamble(
    client_preamble_world: &mut ClientPreambleWorld,
    version: u16,
) -> TestResult {
    if client_preamble_world.server_received_version() != Some(version) {
        return Err(format!(
            "expected server to receive version {}, got {:?}",
            version,
            client_preamble_world.server_received_version()
        )
        .into());
    }
    Ok(())
}

#[then("the client success callback is invoked")]
fn then_success_callback_invoked(client_preamble_world: &mut ClientPreambleWorld) -> TestResult {
    if !client_preamble_world.success_invoked() {
        return Err("expected success callback to be invoked".into());
    }
    client_preamble_world.abort_server();
    Ok(())
}

#[then("the client receives an accepted acknowledgement")]
fn then_receives_ack(client_preamble_world: &mut ClientPreambleWorld) -> TestResult {
    if !client_preamble_world.ack_accepted() {
        return Err("expected client to receive accepted acknowledgement".into());
    }
    client_preamble_world.abort_server();
    Ok(())
}

#[then("the client fails with a timeout error")]
fn then_timeout_error(client_preamble_world: &mut ClientPreambleWorld) -> TestResult {
    if !client_preamble_world.was_timeout_error() {
        return Err("expected timeout error".into());
    }
    client_preamble_world.abort_server();
    Ok(())
}

#[then("the failure callback is invoked")]
fn then_failure_callback_invoked(client_preamble_world: &mut ClientPreambleWorld) -> TestResult {
    if !client_preamble_world.failure_invoked() {
        return Err("expected failure callback to be invoked".into());
    }
    client_preamble_world.abort_server();
    Ok(())
}

#[then("the client connects successfully")]
fn then_connects_successfully(client_preamble_world: &mut ClientPreambleWorld) -> TestResult {
    if !client_preamble_world.is_connected() {
        return Err("expected client to connect successfully".into());
    }
    client_preamble_world.abort_server();
    Ok(())
}
