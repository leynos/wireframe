//! Scenario test functions for derived memory budget defaults.

use rstest_bdd_macros::scenario;

use crate::fixtures::derived_memory_budgets::*;

#[scenario(
    path = "tests/features/derived_memory_budgets.feature",
    name = "Derived budgets enforce per-connection limit"
)]
fn derived_budgets_enforce_per_connection_limit(
    derived_memory_budgets_world: DerivedMemoryBudgetsWorld,
) {
    drop(derived_memory_budgets_world);
}

#[scenario(
    path = "tests/features/derived_memory_budgets.feature",
    name = "Derived budgets allow frames within limits"
)]
fn derived_budgets_allow_frames_within_limits(
    derived_memory_budgets_world: DerivedMemoryBudgetsWorld,
) {
    drop(derived_memory_budgets_world);
}

#[scenario(
    path = "tests/features/derived_memory_budgets.feature",
    name = "Explicit budgets override derived defaults"
)]
fn explicit_budgets_override_derived_defaults(
    derived_memory_budgets_world: DerivedMemoryBudgetsWorld,
) {
    drop(derived_memory_budgets_world);
}
