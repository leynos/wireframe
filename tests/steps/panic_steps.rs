//! Cucumber step implementations for panic resilience testing.
//!
//! Defines Given-When-Then steps that verify server stability
//! when connection tasks panic during setup.

use cucumber::{given, then, when};

use crate::world::{PanicWorld, TestResult};

#[given("a running wireframe server with a panic in connection setup")]
async fn start_server(world: &mut PanicWorld) -> TestResult {
    world.start_panic_server().await?;
    Ok(())
}

#[when("I connect to the server")]
#[when("I connect to the server again")]
async fn connect(world: &mut PanicWorld) -> TestResult {
    world.connect_once().await?;
    Ok(())
}

#[then("both connections succeed")]
async fn verify(world: &mut PanicWorld) -> TestResult {
    world.verify_and_shutdown().await?;
    Ok(())
}
