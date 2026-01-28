//! Scenario tests for connection panic resilience.

use rstest_bdd_macros::scenario;

use crate::fixtures::panic::*;

#[scenario(
    path = "tests/features/connection_panic.feature",
    name = "connection panic does not crash server"
)]
fn panic_resilience(panic_world: PanicWorld) { let _ = panic_world; }
