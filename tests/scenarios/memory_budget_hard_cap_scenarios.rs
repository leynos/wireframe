//! Scenario test functions for hard-cap memory budget connection abort.

use rstest_bdd_macros::scenario;

use crate::fixtures::memory_budget_hard_cap::*;

#[scenario(
    path = "tests/features/memory_budget_hard_cap.feature",
    name = "Connection terminates after repeated budget violations"
)]
fn connection_terminates_after_budget_violations(
    memory_budget_hard_cap_world: MemoryBudgetHardCapWorld,
) {
    drop(memory_budget_hard_cap_world);
}

#[scenario(
    path = "tests/features/memory_budget_hard_cap.feature",
    name = "Connection survives when frames stay within budget"
)]
fn connection_survives_within_budget(memory_budget_hard_cap_world: MemoryBudgetHardCapWorld) {
    drop(memory_budget_hard_cap_world);
}
