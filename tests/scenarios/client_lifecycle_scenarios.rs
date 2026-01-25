//! Scenario tests for client lifecycle hook behaviours.

use rstest_bdd_macros::scenario;

use crate::fixtures::client_lifecycle::*;

#[scenario(
    path = "tests/features/client_lifecycle.feature",
    name = "Setup hook invoked on successful connection"
)]
fn setup_hook_invoked(client_lifecycle_world: ClientLifecycleWorld) {
    let _ = client_lifecycle_world;
}

#[scenario(
    path = "tests/features/client_lifecycle.feature",
    name = "Teardown hook invoked when connection closes"
)]
fn teardown_hook_invoked(client_lifecycle_world: ClientLifecycleWorld) {
    let _ = client_lifecycle_world;
}

#[scenario(
    path = "tests/features/client_lifecycle.feature",
    name = "Error hook invoked on receive failure"
)]
fn error_hook_invoked(client_lifecycle_world: ClientLifecycleWorld) {
    let _ = client_lifecycle_world;
}

#[scenario(
    path = "tests/features/client_lifecycle.feature",
    name = "Lifecycle hooks work with preamble callbacks"
)]
fn lifecycle_with_preamble(client_lifecycle_world: ClientLifecycleWorld) {
    let _ = client_lifecycle_world;
}
