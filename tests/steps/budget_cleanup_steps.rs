//! Step definitions for budget cleanup and reclamation scenarios (8.3.6).

use rstest_bdd_macros::{given, then, when};

use crate::fixtures::budget_cleanup::{BudgetCleanupWorld, CleanupConfig, TestResult};

// ---------------------------------------------------------------------------
// Given
// ---------------------------------------------------------------------------

#[given("a cleanup app configured as {config}")]
fn given_cleanup_app(
    budget_cleanup_world: &mut BudgetCleanupWorld,
    config: CleanupConfig,
) -> TestResult {
    budget_cleanup_world.start_app(config)
}

// ---------------------------------------------------------------------------
// When
// ---------------------------------------------------------------------------

#[when("a cleanup first frame for key {key:u64} with body {body:string} arrives")]
fn when_cleanup_first_frame(
    budget_cleanup_world: &mut BudgetCleanupWorld,
    key: u64,
    body: String,
) -> TestResult {
    budget_cleanup_world.send_first_frame(key, &body)
}

#[when(
    "a cleanup final continuation for key {key:u64} sequence {sequence:u32} with body \
     {body:string} arrives"
)]
fn when_cleanup_final_continuation(
    budget_cleanup_world: &mut BudgetCleanupWorld,
    key: u64,
    sequence: u32,
    body: String,
) -> TestResult {
    budget_cleanup_world.send_final_continuation_frame(key, sequence, &body)
}

#[when("cleanup virtual time advances by {millis:u64} milliseconds")]
fn when_cleanup_virtual_time_advances(
    budget_cleanup_world: &mut BudgetCleanupWorld,
    millis: u64,
) -> TestResult {
    budget_cleanup_world.advance_millis(millis)
}

#[when("the cleanup client disconnects")]
fn when_cleanup_client_disconnects(budget_cleanup_world: &mut BudgetCleanupWorld) {
    budget_cleanup_world.disconnect_client();
}

// ---------------------------------------------------------------------------
// Then
// ---------------------------------------------------------------------------

#[then("cleanup payload {expected:string} is eventually received")]
fn then_cleanup_payload_received(
    budget_cleanup_world: &mut BudgetCleanupWorld,
    expected: String,
) -> TestResult {
    budget_cleanup_world.assert_payload_received(&expected)
}

#[then("no cleanup connection error is recorded")]
fn then_no_cleanup_connection_error(budget_cleanup_world: &mut BudgetCleanupWorld) -> TestResult {
    budget_cleanup_world.assert_no_connection_error()
}
