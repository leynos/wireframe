//! Scenario test functions for memory budget back-pressure.

use rstest_bdd_macros::scenario;

use crate::fixtures::memory_budget_backpressure::*;

#[scenario(
    path = "tests/features/memory_budget_backpressure.feature",
    name = "Soft pressure delays completion until virtual time advances"
)]
fn soft_pressure_delays_completion(
    memory_budget_backpressure_world: MemoryBudgetBackpressureWorld,
) {
    drop(memory_budget_backpressure_world);
}

#[scenario(
    path = "tests/features/memory_budget_backpressure.feature",
    name = "Reads continue without delay when pressure is low"
)]
fn low_pressure_continues_without_delay(
    memory_budget_backpressure_world: MemoryBudgetBackpressureWorld,
) {
    drop(memory_budget_backpressure_world);
}
