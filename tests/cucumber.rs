//! Cucumber test runner for panic resilience integration tests.
//!
//! Runs behavioural tests defined in `tests/features/` using the
//! `PanicWorld` test context to verify server panic handling.

mod steps;
mod world;

use cucumber::World;
use world::PanicWorld;

#[tokio::main]
async fn main() {
    PanicWorld::run("tests/features/connection_panic.feature").await;
}
