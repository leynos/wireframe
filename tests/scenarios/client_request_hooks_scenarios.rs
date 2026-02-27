//! Scenario tests for client request hook behaviours.

use rstest_bdd_macros::scenario;

use crate::fixtures::client_request_hooks::*;

#[scenario(
    path = "tests/features/client_request_hooks.feature",
    name = "Before-send hook is invoked for every outgoing frame"
)]
fn before_send_hook_invoked(client_request_hooks_world: ClientRequestHooksWorld) {
    let _ = client_request_hooks_world;
}

#[scenario(
    path = "tests/features/client_request_hooks.feature",
    name = "After-receive hook is invoked for every incoming frame"
)]
fn after_receive_hook_invoked(client_request_hooks_world: ClientRequestHooksWorld) {
    let _ = client_request_hooks_world;
}

#[scenario(
    path = "tests/features/client_request_hooks.feature",
    name = "Multiple hooks execute in registration order"
)]
fn hooks_execute_in_order(client_request_hooks_world: ClientRequestHooksWorld) {
    let _ = client_request_hooks_world;
}

#[scenario(
    path = "tests/features/client_request_hooks.feature",
    name = "Both hooks fire for a correlated call"
)]
fn both_hooks_fire(client_request_hooks_world: ClientRequestHooksWorld) {
    let _ = client_request_hooks_world;
}

#[scenario(
    path = "tests/features/client_request_hooks.feature",
    name = "Before-send hook can mutate outbound frame bytes"
)]
fn before_send_hook_mutates_frame(client_request_hooks_world: ClientRequestHooksWorld) {
    let _ = client_request_hooks_world;
}

#[scenario(
    path = "tests/features/client_request_hooks.feature",
    name = "After-receive hook can mutate inbound bytes before deserialization"
)]
fn after_receive_hook_mutates_frame(client_request_hooks_world: ClientRequestHooksWorld) {
    let _ = client_request_hooks_world;
}
