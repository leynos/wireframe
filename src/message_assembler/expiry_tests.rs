//! Tests for deadline-indexed assembly expiry (issue #830).
//!
//! These cover staggered deadlines, insertion order, the zero-timeout edge,
//! automatic purging on the continuation path, and safe same-key reuse.

use std::{
    collections::BTreeSet,
    num::NonZeroUsize,
    time::{Duration, Instant},
};

use crate::message_assembler::{
    EnvelopeRouting,
    FirstFrameInput,
    MESSAGE_ASSEMBLY_ENTRY_OVERHEAD_BYTES,
    MessageAssemblyState,
    MessageKey,
    test_helpers::{continuation_header, first_header},
};

/// Build a non-zero message-size or budget limit from a test value.
fn size(value: usize) -> NonZeroUsize {
    NonZeroUsize::new(value).expect("test limit must be non-zero")
}

/// Start a multi-frame assembly for `key` at `now`.
fn start_at(state: &mut MessageAssemblyState, key: u64, body: &[u8], now: Instant) {
    let header = first_header(key, body.len(), false);
    let input = FirstFrameInput::new(&header, EnvelopeRouting::default(), vec![], body)
        .expect("valid first-frame input");
    state
        .accept_first_frame_at(input, now)
        .expect("first frame accepted");
}

/// Collect evicted keys so assertions ignore the index's internal order.
fn evicted_set(state: &mut MessageAssemblyState, now: Instant) -> BTreeSet<MessageKey> {
    state.purge_expired_at(now).into_iter().collect()
}

#[test]
fn purge_evicts_staggered_deadlines_before_at_and_after_timeout() {
    let timeout = Duration::from_secs(30);
    let mut state = MessageAssemblyState::new(size(1024), timeout);
    let base = Instant::now();
    start_at(&mut state, 1, b"", base);
    start_at(&mut state, 2, b"", base + Duration::from_secs(10));
    start_at(&mut state, 3, b"", base + Duration::from_secs(20));

    assert!(
        evicted_set(&mut state, base + Duration::from_secs(29)).is_empty(),
        "nothing expires before the first deadline"
    );
    assert_eq!(
        evicted_set(&mut state, base + Duration::from_secs(30)),
        BTreeSet::from([MessageKey(1)]),
        "the deadline itself is expired"
    );
    assert_eq!(
        evicted_set(&mut state, base + Duration::from_secs(40)),
        BTreeSet::from([MessageKey(2)]),
        "only the second deadline has passed"
    );
    assert_eq!(
        evicted_set(&mut state, base + Duration::from_secs(50)),
        BTreeSet::from([MessageKey(3)]),
        "the last deadline has passed"
    );
    assert_eq!(state.buffered_count(), 0, "every assembly was evicted");
}

#[test]
fn purge_follows_start_times_not_insertion_order() {
    let mut state = MessageAssemblyState::new(size(1024), Duration::from_secs(30));
    let base = Instant::now();
    start_at(&mut state, 1, b"", base + Duration::from_secs(20));
    start_at(&mut state, 2, b"", base);
    start_at(&mut state, 3, b"", base + Duration::from_secs(10));

    assert_eq!(
        evicted_set(&mut state, base + Duration::from_secs(35)),
        BTreeSet::from([MessageKey(2)]),
        "only the earliest start time is expired"
    );
    assert_eq!(state.buffered_count(), 2, "the younger assemblies survive");
}

#[test]
fn zero_timeout_expires_assemblies_started_after_the_sweep() {
    let mut state = MessageAssemblyState::new(size(1024), Duration::ZERO);
    let base = Instant::now();
    start_at(&mut state, 1, b"", base + Duration::from_secs(10));

    let evicted = state.purge_expired_at(base);
    assert_eq!(
        evicted,
        vec![MessageKey(1)],
        "a zero timeout expires every assembly, including a future start"
    );
}

#[test]
fn continuation_acceptance_purges_expired_entries_and_reuses_headroom() {
    let timeout = Duration::from_secs(30);
    let budget = 2 * MESSAGE_ASSEMBLY_ENTRY_OVERHEAD_BYTES + 20;
    let mut state =
        MessageAssemblyState::with_budgets(size(1024), timeout, Some(size(budget)), None);
    let base = Instant::now();

    // Key 2 starts first and is expired by the continuation sweep; key 1
    // starts later and survives with only 11 seconds of age.
    start_at(&mut state, 2, b"0123456789", base);
    start_at(&mut state, 1, b"0123456789", base + Duration::from_secs(20));
    assert_eq!(
        state.total_charged_bytes(),
        2 * MESSAGE_ASSEMBLY_ENTRY_OVERHEAD_BYTES + 20,
        "both entries fill the budget exactly"
    );

    let cont = continuation_header(1, 1, 10, true);
    let message = state
        .accept_continuation_frame_at(&cont, b"abcdefghij", base + Duration::from_secs(31))
        .expect("purging key 2 frees enough headroom")
        .expect("key 1 completes");
    assert_eq!(message.body(), b"0123456789abcdefghij", "body is assembled");
    assert_eq!(state.buffered_count(), 0, "no assembly remains");
    assert_eq!(state.total_buffered_bytes(), 0, "all payload is released");
}

#[test]
fn same_key_is_reusable_after_expiry() {
    let mut state = MessageAssemblyState::new(size(1024), Duration::from_secs(30));
    let base = Instant::now();
    start_at(&mut state, 7, b"old", base);
    assert_eq!(state.total_buffered_bytes(), 3, "old assembly is buffered");

    let evicted = state.purge_expired_at(base + Duration::from_secs(31));
    assert_eq!(evicted, vec![MessageKey(7)], "the old assembly expires");
    assert_eq!(state.total_buffered_bytes(), 0, "its payload is released");

    start_at(&mut state, 7, b"new", base + Duration::from_secs(31));
    assert_eq!(
        state.total_buffered_bytes(),
        3,
        "the same key starts a fresh assembly"
    );
    assert_eq!(state.buffered_count(), 1, "only the fresh assembly exists");
}
