//! Tests for the concurrent-assembly cap and per-entry budget charge.
//!
//! The cap closes the flood vector where a peer starts many cheap, empty
//! assemblies: the fixed charge makes each entry cost budget, and the cap
//! bounds how many can exist even when the payload is empty.

use std::{
    num::NonZeroUsize,
    time::{Duration, Instant},
};

use rstest::rstest;

use super::{submit_first, submit_first_at};
use crate::message_assembler::{
    MESSAGE_ASSEMBLY_ENTRY_OVERHEAD_BYTES,
    MessageAssemblyError,
    MessageAssemblyState,
    MessageKey,
    test_helpers::continuation_header,
};

/// State capped at `cap` concurrent assemblies with no aggregate budgets.
fn capped_state(cap: usize) -> MessageAssemblyState {
    MessageAssemblyState::new(nz!(1024), Duration::from_secs(30))
        .with_max_concurrent_assemblies(Some(non_zero(cap)))
}

/// Build a non-zero limit from a runtime value.
fn non_zero(value: usize) -> NonZeroUsize {
    NonZeroUsize::new(value).expect("test limit must be non-zero")
}

#[rstest]
fn cap_admits_limit_then_rejects_the_next_key() {
    let mut state = capped_state(2);
    submit_first(&mut state, 1, b"", false).expect("first empty assembly");
    submit_first(&mut state, 2, b"", false).expect("second empty assembly");

    let err = submit_first(&mut state, 3, b"", false).expect_err("cap must reject a new key");
    assert_eq!(
        err,
        MessageAssemblyError::TooManyConcurrentAssemblies {
            key: MessageKey(3),
            attempted: 3,
            limit: non_zero(2),
        },
        "rejection must report the key, attempted count and limit"
    );
    assert_eq!(state.buffered_count(), 2, "rejected key is not buffered");
}

#[rstest]
fn cap_preserves_duplicate_key_errors() {
    let mut state = capped_state(1);
    submit_first(&mut state, 1, b"", false).expect("first assembly");

    let err = submit_first(&mut state, 1, b"", false).expect_err("duplicate must be reported");
    assert!(
        matches!(
            err,
            MessageAssemblyError::DuplicateFirstFrame { key: MessageKey(1) }
        ),
        "duplicate key is reported before the cap"
    );
}

#[rstest]
fn single_frame_messages_bypass_the_cap() {
    let mut state = capped_state(1);
    submit_first(&mut state, 1, b"", false).expect("assembly fills the cap");

    let message = submit_first(&mut state, 2, b"single", true)
        .expect("single-frame message bypasses the cap")
        .expect("single-frame message completes immediately");
    assert_eq!(message.body(), b"single", "single-frame body survives");
    assert_eq!(
        state.buffered_count(),
        1,
        "single-frame message is unbuffered"
    );
}

#[rstest]
fn completing_an_assembly_frees_cap_capacity() {
    let mut state = capped_state(1);
    submit_first(&mut state, 1, b"ab", false).expect("first assembly");

    let cont = continuation_header(1, 1, 2, true);
    state
        .accept_continuation_frame(&cont, b"cd")
        .expect("completion is admitted")
        .expect("message completes");

    submit_first(&mut state, 2, b"", false).expect("completed key frees capacity");
    assert_eq!(state.buffered_count(), 1, "the new key is buffered");
}

#[rstest]
fn rejected_continuation_frees_cap_capacity() {
    let mut state = MessageAssemblyState::with_budgets(nz!(4), Duration::from_secs(30), None, None)
        .with_max_concurrent_assemblies(Some(nz!(1)));
    submit_first(&mut state, 1, b"ab", false).expect("first assembly");

    // Five continuation bytes exceed the four-byte per-message limit, so the
    // offending assembly is removed and its cap slot becomes reusable.
    let cont = continuation_header(1, 1, 5, true);
    state
        .accept_continuation_frame(&cont, b"cdefg")
        .expect_err("oversized continuation is rejected");
    assert_eq!(state.buffered_count(), 0, "offending assembly is freed");

    submit_first(&mut state, 2, b"", false).expect("freed capacity is reusable");
}

#[rstest]
fn purge_frees_cap_capacity() {
    let mut state = capped_state(2);
    let now = Instant::now();
    submit_first_at(&mut state, 1, b"", now).expect("first assembly");
    submit_first_at(&mut state, 2, b"", now).expect("second assembly");

    let evicted = state.purge_expired_at(now + Duration::from_secs(31));
    assert_eq!(evicted.len(), 2, "both expired assemblies are evicted");

    submit_first(&mut state, 3, b"", false).expect("purged capacity is reusable");
    submit_first(&mut state, 4, b"", false).expect("both slots are reusable");
}

#[rstest]
fn direct_constructor_state_is_uncapped() {
    let mut state = MessageAssemblyState::new(nz!(1024), Duration::from_secs(30));
    for key in 0_u64..100 {
        submit_first(&mut state, key, b"", false).expect("uncapped state admits every key");
    }
    assert_eq!(
        state.buffered_count(),
        100,
        "all empty assemblies are retained"
    );
}

#[rstest]
fn empty_entries_are_charged_against_the_budget() {
    let mut state = MessageAssemblyState::with_budgets(
        nz!(1024),
        Duration::from_secs(30),
        Some(non_zero(MESSAGE_ASSEMBLY_ENTRY_OVERHEAD_BYTES)),
        None,
    );

    submit_first(&mut state, 1, b"", false).expect("one empty entry exactly fits");
    assert_eq!(
        state.total_buffered_bytes(),
        0,
        "an empty entry retains no payload bytes"
    );
    assert_eq!(
        state.total_charged_bytes(),
        MESSAGE_ASSEMBLY_ENTRY_OVERHEAD_BYTES,
        "an empty entry is charged the fixed overhead"
    );

    let err = submit_first(&mut state, 2, b"", false).expect_err("a second empty entry cannot fit");
    assert!(
        matches!(
            err,
            MessageAssemblyError::ConnectionBudgetExceeded { attempted, .. }
                if attempted == 2 * MESSAGE_ASSEMBLY_ENTRY_OVERHEAD_BYTES
        ),
        "the rejection reports both entries' charges"
    );
}
