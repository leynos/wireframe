//! Step definitions for budget enforcement BDD scenarios (8.3.2).

use rstest_bdd_macros::{given, then, when};

use crate::fixtures::budget_enforcement::{
    BudgetEnforcementWorld,
    BudgetedStateConfig,
    ContinuationInput,
    TestResult,
};

// ---------------------------------------------------------------------------
// Given
// ---------------------------------------------------------------------------

#[given("a budgeted assembly state configured as {config}")]
fn given_budgeted_state(
    budget_enforcement_world: &mut BudgetEnforcementWorld,
    config: BudgetedStateConfig,
) -> TestResult {
    budget_enforcement_world.init_budgeted_state(config)
}

#[given("the clock is at time zero")]
fn given_clock_at_zero(budget_enforcement_world: &mut BudgetEnforcementWorld) {
    budget_enforcement_world.set_epoch();
}

// ---------------------------------------------------------------------------
// When
// ---------------------------------------------------------------------------

#[when("a first frame for key {key} with {body_len} body bytes is accepted")]
fn when_first_frame_accepted(
    budget_enforcement_world: &mut BudgetEnforcementWorld,
    key: u64,
    body_len: usize,
) -> TestResult {
    budget_enforcement_world.accept_first_frame(key, body_len)
}

#[when("a first frame for key {key} with {body_len} body bytes is rejected")]
fn when_first_frame_rejected(
    budget_enforcement_world: &mut BudgetEnforcementWorld,
    key: u64,
    body_len: usize,
) -> TestResult {
    budget_enforcement_world.reject_first_frame(key, body_len)
}

#[when("a first frame for key {key} with {body_len} body bytes is accepted at time zero")]
fn when_first_frame_accepted_at_epoch(
    budget_enforcement_world: &mut BudgetEnforcementWorld,
    key: u64,
    body_len: usize,
) -> TestResult {
    budget_enforcement_world.accept_first_frame_at_epoch(key, body_len)
}

#[when("a continuation for key {key} with sequence {seq} and {body_len} body bytes is rejected")]
fn when_continuation_rejected(
    budget_enforcement_world: &mut BudgetEnforcementWorld,
    key: u64,
    seq: u32,
    body_len: usize,
) -> TestResult {
    budget_enforcement_world.reject_continuation(key, seq, body_len)
}

#[when(
    "a final continuation for key {key} with sequence {seq} and {body_len} body bytes completes \
     the message"
)]
fn when_final_continuation_completes(
    budget_enforcement_world: &mut BudgetEnforcementWorld,
    key: u64,
    seq: u32,
    body_len: usize,
) -> TestResult {
    budget_enforcement_world.accept_continuation(ContinuationInput {
        key,
        sequence: seq,
        body_len,
        is_last: true,
    });
    budget_enforcement_world.assert_completed()
}

#[when("expired assemblies are purged at {offset_secs} seconds")]
fn when_purged_at_offset(
    budget_enforcement_world: &mut BudgetEnforcementWorld,
    offset_secs: u64,
) -> TestResult {
    budget_enforcement_world.purge_at_offset(offset_secs)
}

#[when("a single-frame message for key {key} with {body_len} body bytes is accepted")]
fn when_single_frame_accepted(
    budget_enforcement_world: &mut BudgetEnforcementWorld,
    key: u64,
    body_len: usize,
) -> TestResult {
    budget_enforcement_world.accept_single_frame(key, body_len)
}

// ---------------------------------------------------------------------------
// Then
// ---------------------------------------------------------------------------

#[then("the frame is accepted without error")]
fn then_frame_accepted(budget_enforcement_world: &mut BudgetEnforcementWorld) -> TestResult {
    budget_enforcement_world.assert_accepted()
}

#[then("the error is \"{error_kind}\"")]
fn then_error_is(
    budget_enforcement_world: &mut BudgetEnforcementWorld,
    error_kind: String,
) -> TestResult {
    budget_enforcement_world.assert_error_kind(&error_kind)
}

#[then("total buffered bytes is {expected}")]
fn then_total_buffered_bytes(
    budget_enforcement_world: &mut BudgetEnforcementWorld,
    expected: usize,
) -> TestResult {
    budget_enforcement_world.assert_total_buffered_bytes(expected)
}

#[then("the active assembly count is {expected}")]
fn then_active_assembly_count(
    budget_enforcement_world: &mut BudgetEnforcementWorld,
    expected: usize,
) -> TestResult {
    budget_enforcement_world.assert_buffered_count(expected)
}

#[then("the single-frame message completes immediately")]
fn then_single_frame_completes(
    budget_enforcement_world: &mut BudgetEnforcementWorld,
) -> TestResult {
    budget_enforcement_world.assert_completed()
}
