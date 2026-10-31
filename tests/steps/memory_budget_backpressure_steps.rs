//! Step definitions for soft-limit memory budget back-pressure scenarios.

use rstest_bdd_macros::{given, then, when};

use crate::fixtures::memory_budget_backpressure::{
    BackpressureConfig,
    MemoryBudgetBackpressureWorld,
    TestResult,
};

#[given("a back-pressure inbound app configured as {config}")]
fn given_backpressure_app(
    memory_budget_backpressure_world: &mut MemoryBudgetBackpressureWorld,
    config: BackpressureConfig,
) -> TestResult {
    memory_budget_backpressure_world.start_app(config)
}

#[when("a budgeted first frame for key {key:u64} with body {body:string} arrives")]
fn when_first_frame_arrives(
    memory_budget_backpressure_world: &mut MemoryBudgetBackpressureWorld,
    key: u64,
    body: String,
) -> TestResult {
    memory_budget_backpressure_world.send_first_frame(key, &body)
}

#[when("the budgeted server processes the first frame")]
fn when_server_processes_first_frame(
    memory_budget_backpressure_world: &mut MemoryBudgetBackpressureWorld,
) -> TestResult {
    memory_budget_backpressure_world.wait_for_frame_processed()
}

#[when("the budgeted server parses the next frame")]
fn when_server_parses_next_frame(
    memory_budget_backpressure_world: &mut MemoryBudgetBackpressureWorld,
) -> TestResult {
    memory_budget_backpressure_world.wait_for_frame_processed()
}

#[when(
    "a budgeted final continuation frame for key {key:u64} sequence {sequence:u32} with body \
     {body:string} arrives"
)]
fn when_final_continuation_arrives(
    memory_budget_backpressure_world: &mut MemoryBudgetBackpressureWorld,
    key: u64,
    sequence: u32,
    body: String,
) -> TestResult {
    memory_budget_backpressure_world.send_final_continuation_frame(key, sequence, &body)
}

#[when("ready budgeted payloads are collected")]
fn when_ready_payloads_collected(
    memory_budget_backpressure_world: &mut MemoryBudgetBackpressureWorld,
) -> TestResult {
    memory_budget_backpressure_world.collect_ready_payloads()
}

#[then("no budgeted payload is available before virtual time advances")]
fn then_no_payload_available(
    memory_budget_backpressure_world: &MemoryBudgetBackpressureWorld,
) -> TestResult {
    memory_budget_backpressure_world.assert_no_payload_ready()
}

#[when("budgeted virtual time advances by {millis:u64} milliseconds")]
fn when_virtual_time_advances(
    memory_budget_backpressure_world: &mut MemoryBudgetBackpressureWorld,
    millis: u64,
) -> TestResult {
    memory_budget_backpressure_world.advance_millis(millis)
}

#[then("budgeted payload {expected:string} is eventually received")]
fn then_payload_eventually_received(
    memory_budget_backpressure_world: &mut MemoryBudgetBackpressureWorld,
    expected: String,
) -> TestResult {
    memory_budget_backpressure_world.assert_payload_received(&expected)
}

#[then("no budgeted send error is recorded")]
fn then_no_send_error(
    memory_budget_backpressure_world: &mut MemoryBudgetBackpressureWorld,
) -> TestResult {
    memory_budget_backpressure_world.assert_no_send_error()
}
