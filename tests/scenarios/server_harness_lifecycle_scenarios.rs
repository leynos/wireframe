//! Scenario tests for the protocol-agnostic server lifecycle harness.

use rstest_bdd_macros::scenario;
use wireframe_testing::TestError;

use crate::fixtures::server_harness_lifecycle::{
    ServerHarnessLifecycleWorld,
    server_harness_lifecycle_world,
};

#[scenario(
    path = "tests/features/server_harness_lifecycle.feature",
    name = "A configured server becomes ready and accepts a connection"
)]
fn server_harness_ready_and_accepting(
    server_harness_lifecycle_world: Result<ServerHarnessLifecycleWorld, TestError>,
) -> Result<(), TestError> {
    Ok(())
}

#[scenario(
    path = "tests/features/server_harness_lifecycle.feature",
    name = "Stopping the harness twice is safe"
)]
fn server_harness_stopped_twice(
    server_harness_lifecycle_world: Result<ServerHarnessLifecycleWorld, TestError>,
) -> Result<(), TestError> {
    Ok(())
}

#[scenario(
    path = "tests/features/server_harness_lifecycle.feature",
    name = "Dropping the harness without stopping it releases the port"
)]
fn server_harness_dropped_without_stopping(
    server_harness_lifecycle_world: Result<ServerHarnessLifecycleWorld, TestError>,
) -> Result<(), TestError> {
    Ok(())
}

#[scenario(
    path = "tests/features/server_harness_lifecycle.feature",
    name = "A failing application factory is reported at startup"
)]
fn server_harness_failing_factory(
    server_harness_lifecycle_world: Result<ServerHarnessLifecycleWorld, TestError>,
) -> Result<(), TestError> {
    Ok(())
}
