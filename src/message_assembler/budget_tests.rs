//! Unit tests for aggregate budget enforcement (8.3.2).
//!
//! Helpers, fixtures, and `total_buffered_bytes` accounting tests live here.
//! Enforcement tests (connection, in-flight, dual, isolation, headroom,
//! single-frame bypass) are in the `enforcement` sub-module.

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
    test_helpers::{continuation_header, first_header},
};

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Non-zero shorthand.
fn nz(val: usize) -> NonZeroUsize { NonZeroUsize::new(val).expect("non-zero") }

/// Build a [`FirstFrameHeader`] and [`FirstFrameInput`] in the caller's
/// scope from a key, body slice, and finality flag.
///
/// Encapsulates the header construction and input validation common to all
/// first-frame submission helpers.  The macro expands to two `let` bindings
/// (`$hdr` for the header, `$inp` for the input) so the header lives long
/// enough for the input's borrow.
macro_rules! create_first_frame_input {
    ($hdr:ident, $inp:ident, $key:expr, $body:expr, $is_last:expr) => {
        let $hdr = first_header($key, $body.len(), $is_last);
        let $inp =
            FirstFrameInput::new(&$hdr, EnvelopeRouting::default(), vec![], $body).expect("valid");
    };
}

/// Submit a first frame with `body_len` bytes of body data.
fn submit_first(
    state: &mut MessageAssemblyState,
    key: u64,
    body: &[u8],
    is_last: bool,
) -> Result<Option<crate::message_assembler::AssembledMessage>, MessageAssemblyError> {
    create_first_frame_input!(_header, input, key, body, is_last);
    state.accept_first_frame(input)
}

/// Submit a first frame at a specific timestamp.
fn submit_first_at(
    state: &mut MessageAssemblyState,
    key: u64,
    body: &[u8],
    now: Instant,
) -> Result<Option<crate::message_assembler::AssembledMessage>, MessageAssemblyError> {
    create_first_frame_input!(_header, input, key, body, false);
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
// Budget enforcement tests — see budget_enforcement_tests.rs
// =============================================================================

#[path = "budget_enforcement_tests.rs"]
mod enforcement;
