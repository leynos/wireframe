//! Tests for inbound reassembly ordering, limits, and decoding.

use std::{
    num::NonZeroUsize,
    time::{Duration, Instant},
};

use bincode::{BorrowDecode, Encode};

use crate::fragment::{
    FragmentError,
    FragmentHeader,
    FragmentIndex,
    Fragmenter,
    MessageId,
    ReassembledMessage,
    Reassembler,
    ReassemblyError,
};

fn setup_reassembler_with_first_fragment(
    message_id: u64,
    first_payload: impl AsRef<[u8]>,
) -> Reassembler {
    let mut reassembler = Reassembler::new(
        NonZeroUsize::new(8).expect("non-zero"),
        Duration::from_secs(30),
    );
    let first = FragmentHeader::new(MessageId::new(message_id), FragmentIndex::zero(), false);
    assert!(
        reassembler
            .push(first, first_payload)
            .expect("first fragment accepted")
            .is_none()
    );
    reassembler
}

#[test]
fn reassembler_allows_single_fragment_at_max_message_size() {
    let max_message_size = NonZeroUsize::new(16).expect("non-zero");
    let mut reassembler = Reassembler::new(max_message_size, Duration::from_secs(5));

    let header = FragmentHeader::new(MessageId::new(20), FragmentIndex::zero(), true);
    let payload = vec![0_u8; max_message_size.get()];

    let result = reassembler
        .push(header, payload)
        .expect("fragment within limit should be accepted");

    let assembled = result.expect("single fragment should complete reassembly");
    assert_eq!(assembled.payload().len(), max_message_size.get());
    assert_eq!(reassembler.buffered_len(), 0);
}

#[test]
fn reassembler_allows_multi_fragment_at_max_message_size() {
    let max_message_size = NonZeroUsize::new(16).expect("non-zero");
    let mut reassembler = Reassembler::new(max_message_size, Duration::from_secs(5));

    let first_header = FragmentHeader::new(MessageId::new(21), FragmentIndex::zero(), false);
    let second_header = FragmentHeader::new(MessageId::new(21), FragmentIndex::new(1), true);

    let first_payload = vec![0_u8; 8];
    let second_payload = vec![1_u8; max_message_size.get() - first_payload.len()];

    assert!(
        reassembler
            .push(first_header, first_payload)
            .expect("first fragment within limit")
            .is_none(),
        "first fragment should not complete the message",
    );

    let result = reassembler
        .push(second_header, second_payload)
        .expect("second fragment keeps total at limit");

    let assembled = result.expect("fragments should complete reassembly at exact limit");
    assert_eq!(assembled.payload().len(), max_message_size.get());
    assert_eq!(reassembler.buffered_len(), 0);
}

#[test]
fn reassembler_returns_single_fragment_immediately() {
    let mut reassembler = Reassembler::new(
        NonZeroUsize::new(16).expect("non-zero"),
        Duration::from_secs(5),
    );
    let header = FragmentHeader::new(MessageId::new(1), FragmentIndex::zero(), true);
    let payload = vec![1_u8, 2, 3, 4];

    let complete = reassembler
        .push(header, payload.clone())
        .expect("reassembly must succeed")
        .expect("single fragment should complete message");

    assert_eq!(complete.message_id(), MessageId::new(1));
    assert_eq!(complete.payload(), payload.as_slice());
    assert_eq!(reassembler.buffered_len(), 0);
}

#[test]
fn reassembler_accumulates_ordered_fragments() {
    let mut reassembler = setup_reassembler_with_first_fragment(2, [5_u8, 6, 7]);
    let final_fragment = FragmentHeader::new(MessageId::new(2), FragmentIndex::new(1), true);

    let complete = reassembler
        .push(final_fragment, [8_u8, 9])
        .expect("final fragment accepted")
        .expect("message should complete");

    assert_eq!(complete.payload(), &[5, 6, 7, 8, 9]);
    assert_eq!(reassembler.buffered_len(), 0);
}

#[test]
fn reassembler_rejects_out_of_order_and_drops_partial() {
    let mut reassembler = setup_reassembler_with_first_fragment(3, [1_u8, 2]);
    let skipped = FragmentHeader::new(MessageId::new(3), FragmentIndex::new(2), true);

    let err = reassembler
        .push(skipped, [3_u8])
        .expect_err("out-of-order fragment must be rejected");
    assert!(matches!(
        err,
        ReassemblyError::Fragment(FragmentError::IndexMismatch { .. })
    ));
    assert_eq!(reassembler.buffered_len(), 0);
}

#[test]
fn reassembler_suppresses_duplicate_fragment() {
    let mut reassembler = setup_reassembler_with_first_fragment(31, [1_u8, 2]);
    let duplicate = FragmentHeader::new(MessageId::new(31), FragmentIndex::zero(), false);
    let final_fragment = FragmentHeader::new(MessageId::new(31), FragmentIndex::new(1), true);

    assert!(
        reassembler
            .push(duplicate, [9_u8, 9])
            .expect("duplicate fragment should be suppressed")
            .is_none()
    );
    assert_eq!(reassembler.buffered_len(), 1);

    let complete = reassembler
        .push(final_fragment, [3_u8])
        .expect("final fragment should complete message")
        .expect("message should be complete");
    assert_eq!(complete.payload(), &[1, 2, 3]);
}

#[test]
fn reassembler_accepts_zero_length_fragments() {
    let mut reassembler = Reassembler::new(
        NonZeroUsize::new(8).expect("non-zero"),
        Duration::from_secs(10),
    );
    let first = FragmentHeader::new(MessageId::new(44), FragmentIndex::zero(), false);
    let second = FragmentHeader::new(MessageId::new(44), FragmentIndex::new(1), true);

    assert!(
        reassembler
            .push(first, [])
            .expect("empty fragment should be accepted")
            .is_none()
    );

    let complete = reassembler
        .push(second, [7_u8, 8])
        .expect("final fragment should complete message")
        .expect("message should be complete");
    assert_eq!(complete.payload(), &[7, 8]);
}

#[test]
fn reassembler_enforces_maximum_payload_size() {
    let mut reassembler = Reassembler::new(
        NonZeroUsize::new(4).expect("non-zero"),
        Duration::from_secs(30),
    );
    let first = FragmentHeader::new(MessageId::new(4), FragmentIndex::zero(), false);
    let final_fragment = FragmentHeader::new(MessageId::new(4), FragmentIndex::new(1), true);

    assert!(
        reassembler
            .push(first, [1_u8, 2, 3])
            .expect("first fragment accepted")
            .is_none()
    );

    let err = reassembler
        .push(final_fragment, [4_u8, 5])
        .expect_err("payload growth beyond cap must be rejected");
    assert_eq!(
        err,
        ReassemblyError::MessageTooLarge {
            message_id: MessageId::new(4),
            attempted: 5,
            limit: NonZeroUsize::new(4).expect("non-zero"),
        }
    );
    assert_eq!(reassembler.buffered_len(), 0);
}

#[test]
fn reassembler_purges_expired_messages() {
    let mut reassembler = Reassembler::new(
        NonZeroUsize::new(8).expect("non-zero"),
        Duration::from_secs(2),
    );
    let now = Instant::now();
    let header = FragmentHeader::new(MessageId::new(5), FragmentIndex::zero(), false);

    assert!(
        reassembler
            .push_at(header, [0_u8, 1], now)
            .expect("first fragment accepted")
            .is_none()
    );
    assert_eq!(reassembler.buffered_len(), 1);

    let evicted = reassembler.purge_expired_at(now + Duration::from_secs(3));
    assert_eq!(evicted, vec![MessageId::new(5)]);
    assert_eq!(reassembler.buffered_len(), 0);
}

#[derive(Clone, Debug, Encode, BorrowDecode, PartialEq, Eq)]
struct ExampleMessage(u8);

#[test]
fn reassembler_decodes_reconstructed_message() {
    let fragmenter = Fragmenter::new(NonZeroUsize::new(2).expect("non-zero"));
    let batch = fragmenter
        .fragment_message(&ExampleMessage(11))
        .expect("fragment message");
    let mut reassembler = Reassembler::new(
        NonZeroUsize::new(4).expect("non-zero"),
        Duration::from_secs(10),
    );

    let mut output: Option<ReassembledMessage> = None;
    for fragment in batch {
        let (header, payload) = fragment.into_parts();
        output = reassembler
            .push(header, payload)
            .expect("fragment accepted");
    }

    let assembled = output.expect("message should complete");
    let decoded: ExampleMessage = assembled.decode().expect("decode message");
    assert_eq!(decoded, ExampleMessage(11));
}
