//! Step definitions for the server lifecycle harness behavioural tests.

use rstest_bdd_macros::{given, then, when};

use crate::fixtures::server_harness_lifecycle::{ServerHarnessLifecycleWorld, TestResult};

#[given("a configured Wireframe echo server that has not been bound")]
fn given_unbound_echo_server(server_harness_lifecycle_world: &mut ServerHarnessLifecycleWorld) {
    server_harness_lifecycle_world.given_echo_server();
}

#[given("a configured Wireframe server whose application factory fails")]
fn given_failing_factory(server_harness_lifecycle_world: &mut ServerHarnessLifecycleWorld) {
    server_harness_lifecycle_world.given_failing_factory();
}

#[when("the server lifecycle harness starts the server")]
fn when_harness_starts(server_harness_lifecycle_world: &mut ServerHarnessLifecycleWorld) {
    server_harness_lifecycle_world.start_server();
}

#[when("the harness is stopped twice")]
fn when_stopped_twice(
    server_harness_lifecycle_world: &mut ServerHarnessLifecycleWorld,
) -> TestResult {
    server_harness_lifecycle_world.stop_twice()
}

#[when("the harness is dropped without being stopped")]
fn when_dropped_without_stopping(
    server_harness_lifecycle_world: &mut ServerHarnessLifecycleWorld,
) -> TestResult {
    server_harness_lifecycle_world.drop_without_stopping()
}

#[then("the harness reports a loopback address")]
fn then_loopback_address(
    server_harness_lifecycle_world: &mut ServerHarnessLifecycleWorld,
) -> TestResult {
    server_harness_lifecycle_world.assert_loopback_address()
}

#[then("a client can open a connection to that address")]
fn then_client_can_connect(
    server_harness_lifecycle_world: &mut ServerHarnessLifecycleWorld,
) -> TestResult {
    server_harness_lifecycle_world.assert_client_can_connect()
}

#[then("both stop attempts succeed")]
fn then_both_stops_succeed(
    server_harness_lifecycle_world: &mut ServerHarnessLifecycleWorld,
) -> TestResult {
    server_harness_lifecycle_world.assert_both_stops_succeeded()
}

#[then("the harness reports that it completed")]
fn then_reports_completed(
    server_harness_lifecycle_world: &mut ServerHarnessLifecycleWorld,
) -> TestResult {
    server_harness_lifecycle_world.assert_completed()
}

#[then("the address refuses new connections")]
fn then_refuses_connections(
    server_harness_lifecycle_world: &mut ServerHarnessLifecycleWorld,
) -> TestResult {
    server_harness_lifecycle_world.assert_refuses_connections()
}

#[then("the address stops accepting connections within one second")]
fn then_released_within_one_second(
    server_harness_lifecycle_world: &mut ServerHarnessLifecycleWorld,
) -> TestResult {
    server_harness_lifecycle_world.assert_released_within_one_second()
}

#[then("starting the server fails with an application startup error")]
fn then_application_startup_error(
    server_harness_lifecycle_world: &mut ServerHarnessLifecycleWorld,
) -> TestResult {
    server_harness_lifecycle_world.assert_application_startup_error()
}
