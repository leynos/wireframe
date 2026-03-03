//! Step definitions for budget pressure transition scenarios (8.3.6).

use rstest_bdd_macros::{given, then, when};

use crate::fixtures::budget_transitions::{BudgetTransitionsWorld, TestResult, TransitionConfig};

// ---------------------------------------------------------------------------
// Given
// ---------------------------------------------------------------------------

#[given("a transition app configured as {config}")]
fn given_transition_app(
    budget_transitions_world: &mut BudgetTransitionsWorld,
    config: TransitionConfig,
) -> TestResult {
    budget_transitions_world.start_app(config)
}

// ---------------------------------------------------------------------------
// When
// ---------------------------------------------------------------------------

#[when(
    "transition first frames for keys {start:u64} to {end:u64} each with body {body:string} arrive"
)]
fn when_transition_first_frames_for_range(
    budget_transitions_world: &mut BudgetTransitionsWorld,
    start: u64,
    end: u64,
    body: String,
) -> TestResult {
    budget_transitions_world.send_first_frames_for_range(start, end, &body)
}

#[when("a transition first frame for key {key:u64} with body {body:string} arrives")]
fn when_transition_first_frame(
    budget_transitions_world: &mut BudgetTransitionsWorld,
    key: u64,
    body: String,
) -> TestResult {
    budget_transitions_world.send_first_frame(key, &body)
}

#[when(
    "a transition final continuation for key {key:u64} sequence {sequence:u32} with body \
     {body:string} arrives"
)]
fn when_transition_final_continuation(
    budget_transitions_world: &mut BudgetTransitionsWorld,
    key: u64,
    sequence: u32,
    body: String,
) -> TestResult {
    budget_transitions_world.send_final_continuation_frame(key, sequence, &body)
}

// ---------------------------------------------------------------------------
// Then
// ---------------------------------------------------------------------------

#[then("the transition connection terminates with an error")]
fn then_transition_connection_terminates(
    budget_transitions_world: &mut BudgetTransitionsWorld,
) -> TestResult {
    budget_transitions_world.assert_connection_aborted()
}

#[then("transition payload {expected:string} is eventually received")]
fn then_transition_payload_received(
    budget_transitions_world: &mut BudgetTransitionsWorld,
    expected: String,
) -> TestResult {
    budget_transitions_world.assert_payload_received(&expected)
}

#[then("no transition connection error is recorded")]
fn then_no_transition_connection_error(
    budget_transitions_world: &mut BudgetTransitionsWorld,
) -> TestResult {
    budget_transitions_world.assert_no_connection_error()
}
