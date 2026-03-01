//! Scenario test functions for budget cleanup and reclamation (8.3.6).

use rstest_bdd_macros::scenario;

use crate::fixtures::budget_cleanup::*;

#[scenario(
    path = "tests/features/budget_cleanup.feature",
    name = "Timeout purge reclaims budget for subsequent frames"
)]
fn timeout_purge_reclaims_budget(budget_cleanup_world: BudgetCleanupWorld) {
    drop(budget_cleanup_world);
}

#[scenario(
    path = "tests/features/budget_cleanup.feature",
    name = "Completed assembly reclaims budget for subsequent frames"
)]
fn completed_assembly_reclaims_budget(budget_cleanup_world: BudgetCleanupWorld) {
    drop(budget_cleanup_world);
}

#[scenario(
    path = "tests/features/budget_cleanup.feature",
    name = "Connection close frees all partial assemblies"
)]
fn connection_close_frees_partial_assemblies(budget_cleanup_world: BudgetCleanupWorld) {
    drop(budget_cleanup_world);
}
