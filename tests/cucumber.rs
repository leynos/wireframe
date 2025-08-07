//! Cucumber test runner for integration tests.
//!
//! Orchestrates two distinct test suites:
//! - `PanicWorld`: Tests server resilience during connection panics
//! - `CorrelationWorld`: Tests correlation ID propagation in multi-frame responses
//!
//! # Example
//!
//! The runner executes feature files sequentially:
//! ```text
//! tests/features/connection_panic.feature    -> PanicWorld context
//! tests/features/correlation_id.feature      -> CorrelationWorld context
//! ```
//!
//! Each context provides specialised step definitions and state management
//! for their respective test scenarios.

mod steps;
mod world;

use cucumber::World;
use world::{CorrelationWorld, PanicWorld};

#[tokio::main]
async fn main() {
    PanicWorld::run("tests/features/connection_panic.feature").await;
    CorrelationWorld::run("tests/features/correlation_id.feature").await;
}
