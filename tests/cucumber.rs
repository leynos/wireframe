//! Cucumber test runner for integration tests.
//!
//! Runs behavioural tests defined in `tests/features/` using appropriate
//! test contexts to verify server panic handling and correlation ID
//! propagation.

mod steps;
mod world;

use cucumber::World;
use world::{CorrelationWorld, PanicWorld};

#[tokio::main]
async fn main() {
    PanicWorld::run("tests/features/connection_panic.feature").await;
    CorrelationWorld::run("tests/features/correlation_id.feature").await;
}
