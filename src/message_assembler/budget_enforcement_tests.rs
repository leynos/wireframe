//! Budget enforcement tests: connection, in-flight, dual, isolation,
//! headroom reclamation, and single-frame bypass.

use std::time::{Duration, Instant};

use rstest::rstest;

use super::{
    connection_budgeted_state,
    dual_budgeted_state,
    in_flight_budgeted_state,
    nz,
    submit_first,
    submit_first_at,
    unbounded_state,
};
use crate::message_assembler::{
    MessageAssemblyError,
    MessageAssemblyState,
    MessageKey,
    test_helpers::continuation_header,
};

// =============================================================================
// Helpers for parameterised budget-dimension tests
// =============================================================================

/// Build a `MessageAssemblyState` with only the named budget dimension
/// set to 20 bytes.
fn state_for_dimension(dimension: &str) -> MessageAssemblyState {
    match dimension {
        "connection" => connection_budgeted_state(),
        "in_flight" => in_flight_budgeted_state(),
        _ => panic!("unknown budget dimension: {dimension}"),
    }
}

/// Assert that `err` is the expected budget-exceeded variant for
/// `dimension`, matching the given `key` and `attempted` values.
fn assert_budget_exceeded(err: &MessageAssemblyError, dimension: &str, key: u64, attempted: usize) {
    match dimension {
        "connection" => assert!(
            matches!(
                err,
                MessageAssemblyError::ConnectionBudgetExceeded {
                    key: k,
                    attempted: a,
                    ..
                } if k == &MessageKey(key) && *a == attempted
            ),
            "expected ConnectionBudgetExceeded(key={key}, attempted={attempted}), got: {err:?}"
        ),
        "in_flight" => assert!(
            matches!(
                err,
                MessageAssemblyError::InFlightBudgetExceeded {
                    key: k,
                    attempted: a,
                    ..
                } if k == &MessageKey(key) && *a == attempted
            ),
            "expected InFlightBudgetExceeded(key={key}, attempted={attempted}), got: {err:?}"
        ),
        _ => panic!("unknown budget dimension: {dimension}"),
    }
}

/// Assert that `err` matches the budget-exceeded variant for `dimension`
/// without inspecting field values.
fn assert_budget_exceeded_any(err: &MessageAssemblyError, dimension: &str) {
    match dimension {
        "connection" => assert!(
            matches!(err, MessageAssemblyError::ConnectionBudgetExceeded { .. }),
            "expected ConnectionBudgetExceeded, got: {err:?}"
        ),
        "in_flight" => assert!(
            matches!(err, MessageAssemblyError::InFlightBudgetExceeded { .. }),
            "expected InFlightBudgetExceeded, got: {err:?}"
        ),
        _ => panic!("unknown budget dimension: {dimension}"),
    }
}

// =============================================================================
// Parameterised budget enforcement (connection + in-flight)
// =============================================================================

#[rstest]
#[case::connection("connection")]
#[case::in_flight("in_flight")]
fn budget_allows_frames_within_limit(#[case] dimension: &str) {
    let mut state = state_for_dimension(dimension);
    // 10 bytes fits within 20-byte budget
    submit_first(&mut state, 1, &[0u8; 10], false).expect("within budget");
    assert_eq!(state.total_buffered_bytes(), 10);
}

#[rstest]
#[case::connection("connection")]
#[case::in_flight("in_flight")]
fn budget_rejects_first_frame_exceeding_limit(#[case] dimension: &str) {
    let mut state = state_for_dimension(dimension);
    // 21 bytes exceeds 20-byte budget
    let err = submit_first(&mut state, 1, &[0u8; 21], false).expect_err("should reject");
    assert_budget_exceeded(&err, dimension, 1, 21);
    assert_eq!(state.buffered_count(), 0);
}

#[rstest]
#[case::connection("connection")]
#[case::in_flight("in_flight")]
fn budget_rejects_continuation_exceeding_limit(#[case] dimension: &str) {
    let mut state = state_for_dimension(dimension);
    // First frame: 10 bytes (within 20-byte budget)
    submit_first(&mut state, 1, &[0u8; 10], false).expect("first frame");

    // Continuation: 11 bytes would bring total to 21, exceeding 20
    let cont = continuation_header(1, 1, 11, true);
    let err = state
        .accept_continuation_frame(&cont, &[0u8; 11])
        .expect_err("should reject");
    assert_budget_exceeded_any(&err, dimension);
    // Partial assembly should be freed on budget violation
    assert_eq!(state.buffered_count(), 0);
}

// =============================================================================
// Connection-specific: partial assembly freed on violation
// =============================================================================

#[rstest]
fn connection_budget_frees_partial_on_violation(
    #[from(connection_budgeted_state)] mut state: MessageAssemblyState,
) {
    // Start two assemblies: 8 + 8 = 16 (within 20)
    submit_first(&mut state, 1, &[0u8; 8], false).expect("first");
    submit_first(&mut state, 2, &[0u8; 8], false).expect("second");
    assert_eq!(state.buffered_count(), 2);

    // Continuation on key 1: 5 bytes would bring total to 21
    let cont = continuation_header(1, 1, 5, false);
    let err = state
        .accept_continuation_frame(&cont, &[0u8; 5])
        .expect_err("should reject");
    assert!(matches!(
        err,
        MessageAssemblyError::ConnectionBudgetExceeded { .. }
    ));

    // Key 1 is freed; key 2 survives
    assert_eq!(state.buffered_count(), 1);
    assert_eq!(state.total_buffered_bytes(), 8);
}

// =============================================================================
// Dual budget: tighter budget triggers first
// =============================================================================

#[rstest]
fn dual_budget_in_flight_triggers_before_connection(
    #[from(dual_budgeted_state)] mut state: MessageAssemblyState,
) {
    // In-flight budget is 20, connection budget is 30.
    // 21 bytes should trigger in-flight first.
    let err = submit_first(&mut state, 1, &[0u8; 21], false).expect_err("should reject");
    assert!(
        matches!(err, MessageAssemblyError::InFlightBudgetExceeded { .. }),
        "expected in-flight to trigger first, got: {err:?}"
    );
}

#[test]
fn dual_budget_connection_triggers_when_in_flight_not_exceeded() {
    // connection=15 < in_flight=20, so connection triggers first.
    let mut state = MessageAssemblyState::with_budgets(
        nz(1024),
        Duration::from_secs(30),
        Some(nz(15)),
        Some(nz(20)),
    );
    let err = submit_first(&mut state, 1, &[0u8; 16], false).expect_err("should reject");
    assert!(
        matches!(err, MessageAssemblyError::ConnectionBudgetExceeded { .. }),
        "expected connection to trigger first, got: {err:?}"
    );
}

// =============================================================================
// Backward compatibility: no budgets = no enforcement
// =============================================================================

#[rstest]
fn no_budgets_allows_large_frames(#[from(unbounded_state)] mut state: MessageAssemblyState) {
    // 500 bytes is well within the 1024 per-message limit and has no
    // aggregate cap.
    submit_first(&mut state, 1, &[0u8; 500], false).expect("no budget enforcement");
    submit_first(&mut state, 2, &[0u8; 500], false).expect("no budget enforcement");
    assert_eq!(state.total_buffered_bytes(), 1000);
}

// =============================================================================
// Budget isolation: violation for one key does not affect others
// =============================================================================

#[rstest]
fn budget_violation_does_not_affect_other_assemblies(
    #[from(connection_budgeted_state)] mut state: MessageAssemblyState,
) {
    // Start key 1 with 10 bytes
    submit_first(&mut state, 1, &[0u8; 10], false).expect("first");
    assert_eq!(state.buffered_count(), 1);

    // Try key 2 with 11 bytes — total would be 21, exceeding 20-byte
    // budget
    let err = submit_first(&mut state, 2, &[0u8; 11], false).expect_err("should reject");
    assert!(matches!(
        err,
        MessageAssemblyError::ConnectionBudgetExceeded { .. }
    ));

    // Key 1 is still intact
    assert_eq!(state.buffered_count(), 1);
    assert_eq!(state.total_buffered_bytes(), 10);

    // Key 1 can still be completed
    let cont = continuation_header(1, 1, 3, true);
    let msg = state
        .accept_continuation_frame(&cont, b"fin")
        .expect("cont")
        .expect("complete");
    assert_eq!(msg.body(), &[&[0u8; 10][..], b"fin"].concat());
}

// =============================================================================
// Headroom reclamation after purge
// =============================================================================

#[test]
fn headroom_reclaimed_after_purge_allows_new_assembly() {
    let mut state =
        MessageAssemblyState::with_budgets(nz(1024), Duration::from_secs(30), Some(nz(20)), None);

    let now = Instant::now();
    submit_first_at(&mut state, 1, &[0u8; 15], now).expect("first");
    assert_eq!(state.total_buffered_bytes(), 15);

    // A 6-byte frame would exceed budget (15 + 6 = 21 > 20)
    let err = submit_first(&mut state, 2, &[0u8; 6], false).expect_err("over budget");
    assert!(matches!(
        err,
        MessageAssemblyError::ConnectionBudgetExceeded { .. }
    ));

    // Purge key 1 via timeout
    let future = now + Duration::from_secs(31);
    state.purge_expired_at(future);
    assert_eq!(state.total_buffered_bytes(), 0);

    // Now 6 bytes is fine
    submit_first(&mut state, 3, &[0u8; 6], false).expect("within reclaimed budget");
    assert_eq!(state.total_buffered_bytes(), 6);
}

#[test]
fn headroom_reclaimed_after_completion_allows_new_frame() {
    let mut state =
        MessageAssemblyState::with_budgets(nz(1024), Duration::from_secs(30), None, Some(nz(20)));

    submit_first(&mut state, 1, &[0u8; 15], false).expect("first");

    // Complete key 1
    let cont = continuation_header(1, 1, 3, true);
    state
        .accept_continuation_frame(&cont, b"end")
        .expect("cont")
        .expect("complete");
    assert_eq!(state.total_buffered_bytes(), 0);

    // Now we have full headroom again
    submit_first(&mut state, 2, &[0u8; 20], false).expect("within reclaimed budget");
    assert_eq!(state.total_buffered_bytes(), 20);
}

// =============================================================================
// Single-frame messages bypass aggregate budgets
// =============================================================================

#[rstest]
fn single_frame_message_not_subject_to_aggregate_budgets(
    #[from(connection_budgeted_state)] mut state: MessageAssemblyState,
) {
    // Fill up to the budget edge
    submit_first(&mut state, 1, &[0u8; 19], false).expect("first");
    assert_eq!(state.total_buffered_bytes(), 19);

    // A single-frame message should still succeed because it never
    // buffers
    let msg = submit_first(&mut state, 2, b"big-single-frame-payload", true)
        .expect("single frame accepted")
        .expect("should complete immediately");
    assert_eq!(msg.body(), b"big-single-frame-payload");

    // Buffered bytes unchanged (single-frame was never buffered)
    assert_eq!(state.total_buffered_bytes(), 19);
}
