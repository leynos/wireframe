//! Step definitions for derived memory budget default scenarios.

use rstest_bdd_macros::{given, then, when};

use crate::fixtures::derived_memory_budgets::{
    DerivedMemoryBudgetsWorld,
    ExplicitBudgetConfig,
    TestResult,
};

#[given("a derived-budget app with buffer capacity {capacity:u64}")]
fn given_derived_budget_app(
    derived_memory_budgets_world: &mut DerivedMemoryBudgetsWorld,
    capacity: u64,
) -> TestResult {
    let capacity = usize::try_from(capacity)?;
    derived_memory_budgets_world.start_app_derived(capacity)
}

#[given("a derived-budget app with buffer capacity {capacity:u64} and explicit budgets {config}")]
fn given_derived_budget_app_explicit(
    derived_memory_budgets_world: &mut DerivedMemoryBudgetsWorld,
    capacity: u64,
    config: ExplicitBudgetConfig,
) -> TestResult {
    let capacity = usize::try_from(capacity)?;
    derived_memory_budgets_world.start_app_explicit(capacity, config)
}

#[when(
    "derived-budget first frames for keys {start:u64} to {end:u64} each with body {body:string} \
     arrive"
)]
fn when_derived_budget_first_frames_for_range(
    derived_memory_budgets_world: &mut DerivedMemoryBudgetsWorld,
    start: u64,
    end: u64,
    body: String,
) -> TestResult {
    derived_memory_budgets_world.send_first_frames_for_range(start, end, &body)
}

#[when(
    "derived-budget first frames for keys {start:u64} to {end:u64} each with body size {size:u64} \
     arrive"
)]
fn when_derived_budget_first_frames_for_range_sized(
    derived_memory_budgets_world: &mut DerivedMemoryBudgetsWorld,
    start: u64,
    end: u64,
    size: u64,
) -> TestResult {
    let body = "a".repeat(usize::try_from(size)?);
    derived_memory_budgets_world.send_first_frames_for_range(start, end, &body)
}

#[when("a derived-budget first frame for key {key:u64} with body {body:string} arrives")]
fn when_derived_budget_first_frame(
    derived_memory_budgets_world: &mut DerivedMemoryBudgetsWorld,
    key: u64,
    body: String,
) -> TestResult {
    derived_memory_budgets_world.send_first_frame(key, &body)
}

#[when(
    "a derived-budget final continuation for key {key:u64} sequence {sequence:u32} with body \
     {body:string} arrives"
)]
fn when_derived_budget_final_continuation(
    derived_memory_budgets_world: &mut DerivedMemoryBudgetsWorld,
    key: u64,
    sequence: u32,
    body: String,
) -> TestResult {
    derived_memory_budgets_world.send_final_continuation_frame(key, sequence, &body)
}

#[then("the derived-budget connection terminates with an error")]
fn then_derived_budget_connection_terminates(
    derived_memory_budgets_world: &mut DerivedMemoryBudgetsWorld,
) -> TestResult {
    derived_memory_budgets_world.assert_connection_aborted()
}

#[then("derived-budget payload {expected:string} is eventually received")]
fn then_derived_budget_payload_received(
    derived_memory_budgets_world: &mut DerivedMemoryBudgetsWorld,
    expected: String,
) -> TestResult {
    derived_memory_budgets_world.assert_payload_received(&expected)
}

#[then("no derived-budget connection error is recorded")]
fn then_no_derived_budget_connection_error(
    derived_memory_budgets_world: &mut DerivedMemoryBudgetsWorld,
) -> TestResult {
    derived_memory_budgets_world.assert_no_connection_error()
}
