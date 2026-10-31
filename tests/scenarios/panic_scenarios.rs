//! Scenario tests for connection panic resilience.

use rstest_bdd_macros::scenario;
use wireframe_testing::TestError;

use crate::fixtures::panic::*;

#[scenario(
    path = "tests/features/connection_panic.feature",
    name = "connection panic does not crash server"
)]
fn panic_resilience(panic_world: Result<PanicWorld, TestError>) -> Result<(), TestError> { Ok(()) }
