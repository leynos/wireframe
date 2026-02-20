//! Scenario tests for memory budget builder configuration.

use rstest_bdd_macros::scenario;

use crate::fixtures::memory_budgets::*;

#[scenario(
    path = "tests/features/memory_budgets.feature",
    name = "Create memory budgets with explicit byte caps"
)]
fn create_memory_budgets(memory_budgets_world: MemoryBudgetsWorld) { let _ = memory_budgets_world; }

#[scenario(
    path = "tests/features/memory_budgets.feature",
    name = "Configure a Wireframe app with memory budgets"
)]
fn configure_app_with_memory_budgets(memory_budgets_world: MemoryBudgetsWorld) {
    let _ = memory_budgets_world;
}

#[scenario(
    path = "tests/features/memory_budgets.feature",
    name = "Compose memory budgets with codec configuration"
)]
fn configure_app_with_codec(memory_budgets_world: MemoryBudgetsWorld) {
    let _ = memory_budgets_world;
}

#[scenario(
    path = "tests/features/memory_budgets.feature",
    name = "Reject zero message-byte budget"
)]
fn reject_zero_message_budget(memory_budgets_world: MemoryBudgetsWorld) {
    let _ = memory_budgets_world;
}
