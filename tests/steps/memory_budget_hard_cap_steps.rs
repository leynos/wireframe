//! Step definitions for hard-cap memory budget connection abort scenarios.

use rstest_bdd_macros::{given, then, when};

use crate::fixtures::memory_budget_hard_cap::{
    HardCapConfig,
    MemoryBudgetHardCapWorld,
    TestResult,
};

#[given("a hard-cap inbound app configured as {config}")]
fn given_hard_cap_app(
    memory_budget_hard_cap_world: &mut MemoryBudgetHardCapWorld,
    config: HardCapConfig,
) -> TestResult {
    memory_budget_hard_cap_world.start_app(config)
}

#[when(
    "hard-cap first frames for keys {start:u64} to {end:u64} each with body {body:string} arrive"
)]
fn when_first_frames_for_range(
    memory_budget_hard_cap_world: &mut MemoryBudgetHardCapWorld,
    start: u64,
    end: u64,
    body: String,
) -> TestResult {
    memory_budget_hard_cap_world.send_first_frames_for_range(start, end, &body)
}

#[when("a hard-cap first frame for key {key:u64} with body {body:string} arrives")]
fn when_hard_cap_first_frame(
    memory_budget_hard_cap_world: &mut MemoryBudgetHardCapWorld,
    key: u64,
    body: String,
) -> TestResult {
    memory_budget_hard_cap_world.send_first_frame(key, &body)
}

#[when(
    "a hard-cap final continuation for key {key:u64} sequence {sequence:u32} with body \
     {body:string} arrives"
)]
fn when_hard_cap_final_continuation(
    memory_budget_hard_cap_world: &mut MemoryBudgetHardCapWorld,
    key: u64,
    sequence: u32,
    body: String,
) -> TestResult {
    memory_budget_hard_cap_world.send_final_continuation_frame(key, sequence, &body)
}

#[then("the hard-cap connection terminates with an error")]
fn then_connection_terminates(
    memory_budget_hard_cap_world: &mut MemoryBudgetHardCapWorld,
) -> TestResult {
    memory_budget_hard_cap_world.assert_connection_aborted()
}

#[then("hard-cap payload {expected:string} is eventually received")]
fn then_hard_cap_payload_received(
    memory_budget_hard_cap_world: &mut MemoryBudgetHardCapWorld,
    expected: String,
) -> TestResult {
    memory_budget_hard_cap_world.assert_payload_received(&expected)
}

#[then("no hard-cap connection error is recorded")]
fn then_no_hard_cap_connection_error(
    memory_budget_hard_cap_world: &mut MemoryBudgetHardCapWorld,
) -> TestResult {
    memory_budget_hard_cap_world.assert_no_connection_error()
}
