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

/// One non-final fragment pushed by a rejection scenario.
#[derive(Clone, Copy)]
struct FragmentPush {
    message_id: u64,
    index: u32,
    payload: &'static [u8],
}

impl FragmentPush {
    const fn new(message_id: u64, index: u32, payload: &'static [u8]) -> Self {
        Self {
            message_id,
            index,
            payload,
        }
    }

    fn header(self) -> FragmentHeader {
        FragmentHeader::new(
            MessageId::new(self.message_id),
            FragmentIndex::new(self.index),
            false,
        )
    }
}

/// Aggregate-budget rejection scenario: admitted fragments, then one rejected.
struct RejectionScenario {
    max_message_size: usize,
    max_concurrent_partials: Option<usize>,
    max_buffered_bytes: Option<usize>,
    /// Fragments the budget must admit before the rejected push.
    admitted: Vec<FragmentPush>,
    /// Fragment the budget must reject.
    rejected: FragmentPush,
    /// Error the budget must report for the rejected fragment.
    expected: ReassemblyError,
    /// Partial messages retained after the rejection.
    retained_partials: usize,
    /// Payload bytes retained after the rejection.
    retained_bytes: usize,
}

#[rstest]
#[case::concurrent_cap_rejects_new_message(RejectionScenario {
    max_message_size: 8,
    max_concurrent_partials: Some(1),
    max_buffered_bytes: None,
    admitted: vec![FragmentPush::new(70, 0, &[1, 2])],
    rejected: FragmentPush::new(71, 0, &[3]),
    expected: ReassemblyError::TooManyConcurrentPartials {
        attempted: 2,
        limit: NonZeroUsize::new(1).expect("non-zero limit"),
    },
    retained_partials: 1,
    retained_bytes: 2,
})]
#[case::aggregate_cap_rejects_new_partial(RejectionScenario {
    max_message_size: 8,
    max_concurrent_partials: Some(3),
    max_buffered_bytes: Some(5),
    admitted: vec![
        FragmentPush::new(72, 0, &[1, 2, 3]),
        FragmentPush::new(73, 0, &[4, 5]),
    ],
    rejected: FragmentPush::new(74, 0, &[6]),
    expected: ReassemblyError::AggregateBufferCapExceeded {
        attempted: 6,
        limit: NonZeroUsize::new(5).expect("non-zero limit"),
    },
    retained_partials: 2,
    retained_bytes: 5,
})]
#[case::aggregate_cap_reclaims_after_append(RejectionScenario {
    max_message_size: 8,
    max_concurrent_partials: None,
    max_buffered_bytes: Some(5),
    admitted: vec![FragmentPush::new(75, 0, &[1, 2, 3])],
    rejected: FragmentPush::new(75, 1, &[4, 5, 6]),
    expected: ReassemblyError::AggregateBufferCapExceeded {
        attempted: 6,
        limit: NonZeroUsize::new(5).expect("non-zero limit"),
    },
    retained_partials: 0,
    retained_bytes: 0,
})]
fn rejected_fragment_preserves_retained_budget(#[case] scenario: RejectionScenario) {
    let mut reassembler = reassembler_with_caps(
        scenario.max_message_size,
        scenario.max_concurrent_partials,
        scenario.max_buffered_bytes,
    );
    for fragment in &scenario.admitted {
        reassembler
            .push(fragment.header(), fragment.payload)
            .expect("admitted fragment should fit the budget");
    }

    let error = reassembler
        .push(scenario.rejected.header(), scenario.rejected.payload)
        .expect_err("fragment budget should reject the push");

    assert_eq!(error, scenario.expected);
    assert_eq!(
        (reassembler.buffered_len(), reassembler.buffered_bytes()),
        (scenario.retained_partials, scenario.retained_bytes),
    );
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
    assert_eq!(evicted.len(), 2);
    assert!(evicted.contains(&MessageId::new(76)));
    assert!(evicted.contains(&MessageId::new(77)));
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
