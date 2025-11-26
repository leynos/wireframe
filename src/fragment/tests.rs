use std::{
    num::NonZeroUsize,
    time::{Duration, Instant},
};

use bincode::{BorrowDecode, Encode};
use rstest::rstest;

use super::*;

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
fn fragment_header_exposes_fields() {
    let header = FragmentHeader::new(MessageId::new(9), FragmentIndex::new(2), true);
    assert_eq!(header.message_id(), MessageId::new(9));
    assert_eq!(header.fragment_index(), FragmentIndex::new(2));
    assert!(header.is_last_fragment());
}

#[rstest]
#[case(1)]
#[case(5)]
fn series_accepts_sequential_fragments(#[case] message: u64) {
    let mut series = FragmentSeries::new(MessageId::new(message));
    let first = FragmentHeader::new(MessageId::new(message), FragmentIndex::zero(), false);
    let second = FragmentHeader::new(MessageId::new(message), FragmentIndex::new(1), true);

    assert_eq!(series.accept(first), Ok(FragmentStatus::Incomplete));
    assert_eq!(series.accept(second), Ok(FragmentStatus::Complete));
    assert!(series.is_complete());
}

#[test]
fn series_rejects_other_message() {
    let mut series = FragmentSeries::new(MessageId::new(7));
    let header = FragmentHeader::new(MessageId::new(8), FragmentIndex::zero(), false);
    let err = series
        .accept(header)
        .expect_err("fragment from another message must be rejected");
    assert!(matches!(err, FragmentError::MessageMismatch { .. }));
}

#[test]
fn series_rejects_out_of_order_fragment() {
    let mut series = FragmentSeries::new(MessageId::new(7));
    let header = FragmentHeader::new(MessageId::new(7), FragmentIndex::new(2), false);
    let err = series
        .accept(header)
        .expect_err("out-of-order fragment must be rejected");
    assert!(matches!(err, FragmentError::IndexMismatch { .. }));
}

#[test]
fn series_rejects_after_completion() {
    let mut series = FragmentSeries::new(MessageId::new(1));
    let first = FragmentHeader::new(MessageId::new(1), FragmentIndex::zero(), true);
    assert_eq!(series.accept(first), Ok(FragmentStatus::Complete));
    let err = series
        .accept(FragmentHeader::new(
            MessageId::new(1),
            FragmentIndex::new(1),
            true,
        ))
        .expect_err("series must reject fragments after completion");
    assert!(matches!(err, FragmentError::SeriesComplete));
}

#[test]
fn series_detects_index_overflow() {
    let mut series = FragmentSeries::new(MessageId::new(1));
    series.force_next_index_for_tests(FragmentIndex::new(u32::MAX));
    let header = FragmentHeader::new(MessageId::new(1), FragmentIndex::new(u32::MAX), false);
    let err = series
        .accept(header)
        .expect_err("overflow must raise an error");
    assert!(matches!(err, FragmentError::IndexOverflow { .. }));
}

#[test]
fn series_accepts_final_fragment_at_max_index() {
    let mut series = FragmentSeries::new(MessageId::new(2));
    series.force_next_index_for_tests(FragmentIndex::new(u32::MAX));
    let header = FragmentHeader::new(MessageId::new(2), FragmentIndex::new(u32::MAX), true);
    assert_eq!(series.accept(header), Ok(FragmentStatus::Complete));
    assert!(series.is_complete());
}

#[test]
fn fragmenter_splits_payload_into_multiple_frames() {
    let fragmenter = Fragmenter::new(NonZeroUsize::new(3).expect("non-zero"));
    let payload: Vec<u8> = (0..8).collect();
    let batch = fragmenter
        .fragment_bytes(payload)
        .expect("fragment payload");

    assert_eq!(batch.len(), 3);
    assert!(batch.is_fragmented());
    assert_eq!(batch.message_id(), MessageId::new(0));

    let fragments = batch.fragments();
    assert_eq!(fragments[0].payload(), &[0, 1, 2]);
    assert!(!fragments[0].header().is_last_fragment());
    assert_eq!(fragments[1].payload(), &[3, 4, 5]);
    assert!(!fragments[1].header().is_last_fragment());
    assert_eq!(fragments[2].payload(), &[6, 7]);
    assert!(fragments[2].header().is_last_fragment());
}

#[test]
fn fragmenter_handles_empty_payload() {
    let fragmenter = Fragmenter::new(NonZeroUsize::new(8).expect("non-zero"));
    let batch = fragmenter.fragment_bytes([]).expect("fragment empty");

    assert_eq!(batch.len(), 1);
    assert!(!batch.is_fragmented());
    let fragment = &batch.fragments()[0];
    assert_eq!(fragment.payload(), &[]);
    assert!(fragment.header().is_last_fragment());
    assert_eq!(fragment.header().fragment_index(), FragmentIndex::zero());
}

#[derive(Debug, Encode, BorrowDecode)]
struct DummyMessage(Vec<u8>);

#[test]
fn fragmenter_fragments_messages_and_increments_ids() {
    let fragmenter =
        Fragmenter::with_starting_id(NonZeroUsize::new(4).expect("non-zero"), MessageId::new(7));

    let batch = fragmenter
        .fragment_message(&DummyMessage(vec![1, 2, 3, 4, 5]))
        .expect("fragment message");
    assert_eq!(batch.message_id(), MessageId::new(7));
    assert_eq!(batch.len(), 2);
    assert!(batch.is_fragmented());

    let next_payload = vec![9, 9, 9];
    let next = fragmenter
        .fragment_bytes(next_payload)
        .expect("fragment bytes");
    assert_eq!(next.message_id(), MessageId::new(8));
    assert_eq!(next.len(), 1);
    assert!(!next.is_fragmented());
}

#[test]
fn fragment_batch_into_iterator_yields_all_fragments() {
    let fragmenter = Fragmenter::new(NonZeroUsize::new(2).expect("non-zero"));
    let payload = [1_u8, 2, 3];
    let batch = fragmenter
        .fragment_bytes(payload)
        .expect("split into fragments");

    let payloads: Vec<Vec<u8>> = batch
        .into_iter()
        .map(|fragment| fragment.payload().to_vec())
        .collect();
    assert_eq!(payloads, vec![vec![1, 2], vec![3]]);
}

#[test]
fn fragmenter_respects_explicit_message_ids() {
    let fragmenter =
        Fragmenter::with_starting_id(NonZeroUsize::new(2).expect("non-zero"), MessageId::new(10));
    let payload = [7_u8, 8, 9];
    let batch = fragmenter
        .fragment_with_id(MessageId::new(500), payload)
        .expect("fragment with explicit id");
    assert_eq!(batch.message_id(), MessageId::new(500));
    assert_eq!(batch.len(), 2);

    let next = fragmenter.fragment_bytes([1_u8]).expect("next fragment");
    assert_eq!(next.message_id(), MessageId::new(10));
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

    let mut output = None;
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
