//! Cucumber test runner for correlation id behaviour.

mod correlation_world;
#[path = "steps/correlation_steps.rs"]
mod steps;

use correlation_world::CorrelationWorld;
use cucumber::World;

#[tokio::main]
async fn main() {
    CorrelationWorld::run("tests/features/correlation").await;
}
