//! Step definitions for memory budget builder scenarios.

use rstest_bdd_macros::{given, then, when};

use crate::fixtures::memory_budgets::{MemoryBudgetsWorld, StepBudgetBytes, TestResult};

#[given(
    "memory budgets with message bytes {per_message}, connection bytes {per_connection}, and \
     in-flight bytes {in_flight}"
)]
fn given_memory_budgets(
    memory_budgets_world: &mut MemoryBudgetsWorld,
    per_message: StepBudgetBytes,
    per_connection: StepBudgetBytes,
    in_flight: StepBudgetBytes,
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

#[when(
    "attempting to configure memory budgets with message bytes {per_message}, connection bytes \
     {per_connection}, and in-flight bytes {in_flight}"
)]
fn when_attempting_to_configure_memory_budgets(
    memory_budgets_world: &mut MemoryBudgetsWorld,
    per_message: StepBudgetBytes,
    per_connection: StepBudgetBytes,
    in_flight: StepBudgetBytes,
) {
    memory_budgets_world.attempt_set_budgets(per_message, per_connection, in_flight);
}

#[then("the message budget is {expected} bytes")]
fn then_message_budget(
    memory_budgets_world: &mut MemoryBudgetsWorld,
    expected: StepBudgetBytes,
) -> TestResult {
    memory_budgets_world.assert_message_budget(expected)
}

#[then("the connection budget is {expected} bytes")]
fn then_connection_budget(
    memory_budgets_world: &mut MemoryBudgetsWorld,
    expected: StepBudgetBytes,
) -> TestResult {
    memory_budgets_world.assert_connection_budget(expected)
}

#[then("the in-flight budget is {expected} bytes")]
fn then_in_flight_budget(
    memory_budgets_world: &mut MemoryBudgetsWorld,
    expected: StepBudgetBytes,
) -> TestResult {
    memory_budgets_world.assert_in_flight_budget(expected)
}

#[then("app configuration with memory budgets succeeds")]
fn then_app_configuration_succeeds(memory_budgets_world: &mut MemoryBudgetsWorld) -> TestResult {
    memory_budgets_world.assert_configuration_succeeded()
}

#[then("configuring memory budgets fails with error \"{expected_error}\"")]
fn then_configuring_memory_budgets_fails_with_error(
    memory_budgets_world: &mut MemoryBudgetsWorld,
    expected_error: String,
) -> TestResult {
    memory_budgets_world.assert_budget_setup_failed_with(&expected_error)
}
