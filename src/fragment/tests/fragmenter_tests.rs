//! Tests for outbound fragmentation and fragment batch helpers.

use std::num::NonZeroUsize;

use bincode::{BorrowDecode, Encode};

use crate::fragment::{
    FragmentBatch,
    FragmentIndex,
    FragmentationError,
    Fragmenter,
    MessageId,
    fragmenter::FragmentCursor,
};

#[derive(Debug, Encode, BorrowDecode)]
struct DummyMessage(Vec<u8>);

fn assert_fragment(batch: &FragmentBatch, index: usize, payload: &[u8], is_last: bool) {
    let fragment = batch
        .fragments()
        .get(index)
        .expect("fragment missing at requested index");
    assert_eq!(fragment.payload(), payload);
    assert_eq!(fragment.header().is_last_fragment(), is_last);
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

    assert_fragment(&batch, 0, &[0, 1, 2], false);
    assert_fragment(&batch, 1, &[3, 4, 5], false);
    assert_fragment(&batch, 2, &[6, 7], true);
}

#[test]
fn fragmenter_handles_empty_payload() {
    let fragmenter = Fragmenter::new(NonZeroUsize::new(8).expect("non-zero"));
    let batch = fragmenter.fragment_bytes([]).expect("fragment empty");

    assert_eq!(batch.len(), 1);
    assert!(!batch.is_fragmented());
    let fragment = batch
        .fragments()
        .first()
        .expect("batch should contain at least one fragment");
    assert!(fragment.payload().is_empty());
    assert!(fragment.header().is_last_fragment());
    assert_eq!(fragment.header().fragment_index(), FragmentIndex::zero());
}

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
fn fragmenter_returns_error_for_out_of_bounds_slice() {
    let fragmenter = Fragmenter::new(NonZeroUsize::new(4).expect("non-zero"));
    let payload = [1_u8, 2, 3, 4];

    let err = fragmenter
        .build_fragments_from_for_tests(
            MessageId::new(1),
            &payload,
            FragmentCursor::new(payload.len() + 1, FragmentIndex::zero()),
        )
        .expect_err("invalid slice should produce an error");
    match err {
        FragmentationError::SliceBounds { offset, end, total } => {
            assert_eq!(offset, payload.len() + 1);
            assert_eq!(end, payload.len() + 1);
            assert_eq!(total, payload.len());
        }
        other => panic!("expected SliceBounds, got {other:?}"),
    }
}
