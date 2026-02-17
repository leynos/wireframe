//! Step definitions for memory budget builder scenarios.

use rstest_bdd_macros::{given, then, when};

use crate::fixtures::memory_budgets::{BudgetBytes, MemoryBudgetsWorld, TestResult};

#[given(
    "memory budgets with message bytes {per_message:usize}, connection bytes \
     {per_connection:usize}, and in-flight bytes {in_flight:usize}"
)]
fn given_memory_budgets(
    memory_budgets_world: &mut MemoryBudgetsWorld,
    per_message: BudgetBytes,
    per_connection: BudgetBytes,
    in_flight: BudgetBytes,
) -> TestResult {
    memory_budgets_world.set_budgets(per_message, per_connection, in_flight)
}

#[when("configuring a Wireframe app with memory budgets")]
fn when_configuring_app_with_budgets(memory_budgets_world: &mut MemoryBudgetsWorld) -> TestResult {
    memory_budgets_world.configure_app_with_budgets()
}

#[when("configuring a Wireframe app with memory budgets and a custom codec budget")]
fn when_configuring_app_with_budgets_and_codec(
    memory_budgets_world: &mut MemoryBudgetsWorld,
) -> TestResult {
    memory_budgets_world.configure_app_with_budgets_and_codec()
}

#[then("the message budget is {expected:usize} bytes")]
fn then_message_budget(
    memory_budgets_world: &mut MemoryBudgetsWorld,
    expected: BudgetBytes,
) -> TestResult {
    memory_budgets_world.assert_message_budget(expected)
}

#[then("the connection budget is {expected:usize} bytes")]
fn then_connection_budget(
    memory_budgets_world: &mut MemoryBudgetsWorld,
    expected: BudgetBytes,
) -> TestResult {
    memory_budgets_world.assert_connection_budget(expected)
}

#[then("the in-flight budget is {expected:usize} bytes")]
fn then_in_flight_budget(
    memory_budgets_world: &mut MemoryBudgetsWorld,
    expected: BudgetBytes,
) -> TestResult {
    memory_budgets_world.assert_in_flight_budget(expected)
}

#[then("app configuration with memory budgets succeeds")]
fn then_app_configuration_succeeds(memory_budgets_world: &mut MemoryBudgetsWorld) -> TestResult {
    memory_budgets_world.assert_configuration_succeeded()
}
