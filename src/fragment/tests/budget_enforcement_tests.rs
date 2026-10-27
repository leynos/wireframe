//! Tests for per-connection limits on incomplete fragment reassembly.

use std::{
    num::NonZeroUsize,
    time::{Duration, Instant},
};

use rstest::rstest;

use crate::fragment::{FragmentHeader, FragmentIndex, MessageId, Reassembler, ReassemblyError};

fn reassembler_with_caps(
    max_message_size: usize,
    max_concurrent_partials: Option<usize>,
    max_buffered_bytes: Option<usize>,
) -> Reassembler {
    Reassembler::new(
        NonZeroUsize::new(max_message_size).expect("message cap must be non-zero"),
        Duration::from_secs(30),
        max_concurrent_partials
            .map(|limit| NonZeroUsize::new(limit).expect("partial-message cap must be non-zero")),
        max_buffered_bytes
            .map(|limit| NonZeroUsize::new(limit).expect("buffered-byte cap must be non-zero")),
    )
}

#[rstest]
fn concurrent_partial_cap_rejects_a_new_message_without_allocating_it() {
    let mut reassembler = reassembler_with_caps(8, Some(1), None);
    let first = FragmentHeader::new(MessageId::new(70), FragmentIndex::zero(), false);
    let second = FragmentHeader::new(MessageId::new(71), FragmentIndex::zero(), false);

    reassembler
        .push(first, [1_u8, 2])
        .expect("first partial should fit the count cap");
    let error = reassembler
        .push(second, [3_u8])
        .expect_err("a second partial should exceed the count cap");

    assert_eq!(
        error,
        ReassemblyError::TooManyConcurrentPartials {
            attempted: 2,
            limit: NonZeroUsize::new(1).expect("non-zero limit"),
        },
    );
    assert_eq!(reassembler.buffered_len(), 1);
    assert_eq!(reassembler.buffered_bytes(), 2);
}

#[rstest]
fn aggregate_byte_cap_accepts_exact_fit_and_rejects_new_partial_growth() {
    let mut reassembler = reassembler_with_caps(8, Some(3), Some(5));
    let first = FragmentHeader::new(MessageId::new(72), FragmentIndex::zero(), false);
    let second = FragmentHeader::new(MessageId::new(73), FragmentIndex::zero(), false);
    let third = FragmentHeader::new(MessageId::new(74), FragmentIndex::zero(), false);

    reassembler
        .push(first, [1_u8, 2, 3])
        .expect("first partial should fit");
    reassembler
        .push(second, [4_u8, 5])
        .expect("aggregate cap should accept an exact fit");
    assert_eq!(reassembler.buffered_len(), 2);
    assert_eq!(reassembler.buffered_bytes(), 5);

    let error = reassembler
        .push(third, [6_u8])
        .expect_err("new payload should exceed the aggregate byte cap");
    assert_eq!(
        error,
        ReassemblyError::AggregateBufferCapExceeded {
            attempted: 6,
            limit: NonZeroUsize::new(5).expect("non-zero limit"),
        },
    );
    assert_eq!(reassembler.buffered_len(), 2);
    assert_eq!(reassembler.buffered_bytes(), 5);
}

#[rstest]
fn aggregate_byte_cap_rejects_append_growth_and_reclaims_the_partial() {
    let mut reassembler = reassembler_with_caps(8, None, Some(5));
    let first = FragmentHeader::new(MessageId::new(75), FragmentIndex::zero(), false);
    let second = FragmentHeader::new(MessageId::new(75), FragmentIndex::new(1), false);

    reassembler
        .push(first, [1_u8, 2, 3])
        .expect("first fragment should fit");
    let error = reassembler
        .push(second, [4_u8, 5, 6])
        .expect_err("append should exceed the aggregate byte cap");

    assert_eq!(
        error,
        ReassemblyError::AggregateBufferCapExceeded {
            attempted: 6,
            limit: NonZeroUsize::new(5).expect("non-zero limit"),
        },
    );
    assert_eq!(reassembler.buffered_len(), 0);
    assert_eq!(reassembler.buffered_bytes(), 0);
}

#[rstest]
fn aggregate_byte_cap_accepts_an_append_that_exactly_fills_the_limit() {
    let mut reassembler = reassembler_with_caps(8, None, Some(5));
    let first = FragmentHeader::new(MessageId::new(82), FragmentIndex::zero(), false);
    let second = FragmentHeader::new(MessageId::new(82), FragmentIndex::new(1), false);

    reassembler
        .push(first, [1_u8, 2, 3])
        .expect("first fragment should fit");
    reassembler
        .push(second, [4_u8, 5])
        .expect("aggregate cap should accept an exact-fit append");

    assert_eq!(reassembler.buffered_len(), 1);
    assert_eq!(reassembler.buffered_bytes(), 5);
}

#[rstest]
fn purge_reclaims_count_and_byte_capacity_for_later_messages() {
    let mut reassembler = Reassembler::new(
        NonZeroUsize::new(8).expect("message cap"),
        Duration::from_secs(2),
        NonZeroUsize::new(2),
        NonZeroUsize::new(6),
    );
    let now = Instant::now();
    for (message_id, payload) in [(76, &[1_u8, 2][..]), (77, &[3_u8, 4, 5][..])] {
        let first = FragmentHeader::new(MessageId::new(message_id), FragmentIndex::zero(), false);
        reassembler
            .push_at(first, payload, now)
            .expect("partial should fit both caps");
    }
    assert_eq!(reassembler.buffered_len(), 2);
    assert_eq!(reassembler.buffered_bytes(), 5);

    let evicted = reassembler.purge_expired_at(now + Duration::from_secs(3));
    assert_eq!(evicted, vec![MessageId::new(76), MessageId::new(77)]);
    assert_eq!(reassembler.buffered_len(), 0);
    assert_eq!(reassembler.buffered_bytes(), 0);

    let later = FragmentHeader::new(MessageId::new(78), FragmentIndex::zero(), false);
    reassembler
        .push(later, [9_u8; 6])
        .expect("purge should reclaim both caps");
    assert_eq!(reassembler.buffered_len(), 1);
    assert_eq!(reassembler.buffered_bytes(), 6);
}

#[rstest]
fn completion_reclaims_only_the_completed_message_budget() {
    let mut reassembler = reassembler_with_caps(8, Some(2), Some(6));
    let first = FragmentHeader::new(MessageId::new(79), FragmentIndex::zero(), false);
    let other = FragmentHeader::new(MessageId::new(80), FragmentIndex::zero(), false);
    let final_fragment = FragmentHeader::new(MessageId::new(79), FragmentIndex::new(1), true);

    reassembler
        .push(first, [1_u8, 2])
        .expect("first partial should fit");
    reassembler
        .push(other, [3_u8, 4])
        .expect("second message id should be independent");
    let complete = reassembler
        .push(final_fragment, [5_u8, 6])
        .expect("completed message should fit the aggregate cap")
        .expect("final fragment should complete the message");

    assert_eq!(complete.payload(), &[1, 2, 5, 6]);
    assert_eq!(reassembler.buffered_len(), 1);
    assert_eq!(reassembler.buffered_bytes(), 2);
    let third = FragmentHeader::new(MessageId::new(81), FragmentIndex::zero(), false);
    reassembler
        .push(third, [7_u8; 4])
        .expect("remaining aggregate headroom should be available");
    assert_eq!(reassembler.buffered_len(), 2);
    assert_eq!(reassembler.buffered_bytes(), 6);
}
