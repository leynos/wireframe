//! Unit tests for aggregate budget enforcement (8.3.2).

use std::{
    num::NonZeroUsize,
    time::{Duration, Instant},
};

use rstest::{fixture, rstest};

use crate::message_assembler::{
    EnvelopeRouting,
    FirstFrameInput,
    MessageAssemblyError,
    MessageAssemblyState,
    MessageKey,
    test_helpers::{continuation_header, first_header},
};

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Non-zero shorthand.
fn nz(val: usize) -> NonZeroUsize { NonZeroUsize::new(val).expect("non-zero") }

/// Submit a first frame with `body_len` bytes of body data.
fn submit_first(
    state: &mut MessageAssemblyState,
    key: u64,
    body: &[u8],
    is_last: bool,
) -> Result<Option<crate::message_assembler::AssembledMessage>, MessageAssemblyError> {
    let header = first_header(key, body.len(), is_last);
    let input =
        FirstFrameInput::new(&header, EnvelopeRouting::default(), vec![], body).expect("valid");
    state.accept_first_frame(input)
}

/// Submit a first frame at a specific timestamp.
fn submit_first_at(
    state: &mut MessageAssemblyState,
    key: u64,
    body: &[u8],
    now: Instant,
) -> Result<Option<crate::message_assembler::AssembledMessage>, MessageAssemblyError> {
    let header = first_header(key, body.len(), false);
    let input =
        FirstFrameInput::new(&header, EnvelopeRouting::default(), vec![], body).expect("valid");
    state.accept_first_frame_at(input, now)
}

// ---------------------------------------------------------------------------
// Fixtures
// ---------------------------------------------------------------------------

/// State with no budgets — backwards-compatibility baseline.
#[fixture]
fn unbounded_state() -> MessageAssemblyState {
    MessageAssemblyState::new(nz(1024), Duration::from_secs(30))
}

/// State with a 20-byte connection budget and 1024-byte per-message limit.
#[fixture]
fn connection_budgeted_state() -> MessageAssemblyState {
    MessageAssemblyState::with_budgets(nz(1024), Duration::from_secs(30), Some(nz(20)), None)
}

/// State with a 20-byte in-flight budget and 1024-byte per-message limit.
#[fixture]
fn in_flight_budgeted_state() -> MessageAssemblyState {
    MessageAssemblyState::with_budgets(nz(1024), Duration::from_secs(30), None, Some(nz(20)))
}

/// State with both connection (30) and in-flight (20) budgets.
#[fixture]
fn dual_budgeted_state() -> MessageAssemblyState {
    MessageAssemblyState::with_budgets(
        nz(1024),
        Duration::from_secs(30),
        Some(nz(30)),
        Some(nz(20)),
    )
}

// =============================================================================
// total_buffered_bytes accounting
// =============================================================================

#[rstest]
fn total_buffered_bytes_starts_at_zero(#[from(unbounded_state)] state: MessageAssemblyState) {
    assert_eq!(state.total_buffered_bytes(), 0);
}

#[rstest]
fn total_buffered_bytes_tracks_single_assembly(
    #[from(unbounded_state)] mut state: MessageAssemblyState,
) {
    submit_first(&mut state, 1, b"hello", false).expect("first frame");
    assert_eq!(state.total_buffered_bytes(), 5);
}

#[rstest]
fn total_buffered_bytes_tracks_multiple_assemblies(
    #[from(unbounded_state)] mut state: MessageAssemblyState,
) {
    submit_first(&mut state, 1, b"aaa", false).expect("first");
    submit_first(&mut state, 2, b"bbbbb", false).expect("second");
    assert_eq!(state.total_buffered_bytes(), 8); // 3 + 5
}

#[rstest]
fn total_buffered_bytes_decreases_after_completion(
    #[from(unbounded_state)] mut state: MessageAssemblyState,
) {
    submit_first(&mut state, 1, b"aaa", false).expect("first");
    submit_first(&mut state, 2, b"bb", false).expect("second");
    assert_eq!(state.total_buffered_bytes(), 5);

    // Complete key 1
    let cont = continuation_header(1, 1, 2, true);
    state
        .accept_continuation_frame(&cont, b"xx")
        .expect("cont")
        .expect("complete");
    assert_eq!(state.total_buffered_bytes(), 2); // only key 2 remains
}

#[rstest]
fn total_buffered_bytes_decreases_after_purge(
    #[from(unbounded_state)] mut state: MessageAssemblyState,
) {
    let now = Instant::now();
    submit_first_at(&mut state, 1, b"data", now).expect("first");
    assert_eq!(state.total_buffered_bytes(), 4);

    let future = now + Duration::from_secs(31);
    state.purge_expired_at(future);
    assert_eq!(state.total_buffered_bytes(), 0);
}

#[rstest]
fn single_frame_message_not_counted_in_buffered_bytes(
    #[from(unbounded_state)] mut state: MessageAssemblyState,
) {
    submit_first(&mut state, 1, b"complete", true)
        .expect("first")
        .expect("complete");
    assert_eq!(state.total_buffered_bytes(), 0);
    assert_eq!(state.buffered_count(), 0);
}

// =============================================================================
// Connection budget enforcement
// =============================================================================

#[rstest]
fn connection_budget_allows_frames_within_limit(
    #[from(connection_budgeted_state)] mut state: MessageAssemblyState,
) {
    // 10 bytes fits within 20-byte budget
    submit_first(&mut state, 1, &[0u8; 10], false).expect("within budget");
    assert_eq!(state.total_buffered_bytes(), 10);
}

#[rstest]
fn connection_budget_rejects_first_frame_exceeding_limit(
    #[from(connection_budgeted_state)] mut state: MessageAssemblyState,
) {
    // 21 bytes exceeds 20-byte budget
    let err = submit_first(&mut state, 1, &[0u8; 21], false).expect_err("should reject");
    assert!(
        matches!(
            err,
            MessageAssemblyError::ConnectionBudgetExceeded {
                key: MessageKey(1),
                attempted: 21,
                ..
            }
        ),
        "unexpected error: {err:?}"
    );
    assert_eq!(state.buffered_count(), 0);
}

#[rstest]
fn connection_budget_rejects_continuation_exceeding_limit(
    #[from(connection_budgeted_state)] mut state: MessageAssemblyState,
) {
    // First frame: 10 bytes (within 20-byte budget)
    submit_first(&mut state, 1, &[0u8; 10], false).expect("first frame");

    // Continuation: 11 bytes would bring total to 21, exceeding 20
    let cont = continuation_header(1, 1, 11, true);
    let err = state
        .accept_continuation_frame(&cont, &[0u8; 11])
        .expect_err("should reject");
    assert!(
        matches!(err, MessageAssemblyError::ConnectionBudgetExceeded { .. }),
        "unexpected error: {err:?}"
    );
    // Partial assembly should be freed on budget violation
    assert_eq!(state.buffered_count(), 0);
}

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
// In-flight budget enforcement
// =============================================================================

#[rstest]
fn in_flight_budget_allows_frames_within_limit(
    #[from(in_flight_budgeted_state)] mut state: MessageAssemblyState,
) {
    submit_first(&mut state, 1, &[0u8; 10], false).expect("within budget");
    assert_eq!(state.total_buffered_bytes(), 10);
}

#[rstest]
fn in_flight_budget_rejects_first_frame_exceeding_limit(
    #[from(in_flight_budgeted_state)] mut state: MessageAssemblyState,
) {
    let err = submit_first(&mut state, 1, &[0u8; 21], false).expect_err("should reject");
    assert!(
        matches!(
            err,
            MessageAssemblyError::InFlightBudgetExceeded {
                key: MessageKey(1),
                attempted: 21,
                ..
            }
        ),
        "unexpected error: {err:?}"
    );
    assert_eq!(state.buffered_count(), 0);
}

#[rstest]
fn in_flight_budget_rejects_continuation_exceeding_limit(
    #[from(in_flight_budgeted_state)] mut state: MessageAssemblyState,
) {
    submit_first(&mut state, 1, &[0u8; 10], false).expect("first frame");

    let cont = continuation_header(1, 1, 11, true);
    let err = state
        .accept_continuation_frame(&cont, &[0u8; 11])
        .expect_err("should reject");
    assert!(
        matches!(err, MessageAssemblyError::InFlightBudgetExceeded { .. }),
        "unexpected error: {err:?}"
    );
    assert_eq!(state.buffered_count(), 0);
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
    // 500 bytes is well within the 1024 per-message limit and has no aggregate cap.
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

    // Try key 2 with 11 bytes — total would be 21, exceeding 20-byte budget
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

    // A single-frame message should still succeed because it never buffers
    let msg = submit_first(&mut state, 2, b"big-single-frame-payload", true)
        .expect("single frame accepted")
        .expect("should complete immediately");
    assert_eq!(msg.body(), b"big-single-frame-payload");

    // Buffered bytes unchanged (single-frame was never buffered)
    assert_eq!(state.total_buffered_bytes(), 19);
}
