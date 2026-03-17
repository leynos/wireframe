//! Integration tests for `wireframe_testing::reassembly`.

use rstest::rstest;
use wireframe::{
    fragment::{MessageId, ReassembledMessage},
    message_assembler::{
        AssembledMessage,
        EnvelopeId,
        EnvelopeRouting,
        FrameSequence,
        MessageAssemblyError,
        MessageKey,
        MessageSeriesError,
    },
};
use wireframe_testing::reassembly::{
    FragmentReassemblyErrorExpectation,
    FragmentReassemblySnapshot,
    MessageAssemblyErrorExpectation,
    MessageAssemblySnapshot,
    assert_fragment_reassembly_absent,
    assert_fragment_reassembly_buffered_messages,
    assert_fragment_reassembly_completed_len,
    assert_fragment_reassembly_error,
    assert_fragment_reassembly_evicted,
    assert_message_assembly_buffered_count,
    assert_message_assembly_completed,
    assert_message_assembly_completed_for_key,
    assert_message_assembly_error,
    assert_message_assembly_evicted,
    assert_message_assembly_incomplete,
    assert_message_assembly_total_buffered_bytes,
};

fn routing() -> EnvelopeRouting {
    EnvelopeRouting {
        envelope_id: EnvelopeId(1),
        correlation_id: None,
    }
}

fn assembled_message(key: u64, body: &[u8]) -> AssembledMessage {
    AssembledMessage::new(MessageKey(key), routing(), vec![], body.to_vec())
}

#[test]
fn message_assertion_helpers_accept_incomplete_and_counts() {
    let last_result = Ok(None);
    let snapshot = MessageAssemblySnapshot::new(Some(&last_result), &[], &[], 1, 5);

    assert_message_assembly_incomplete(snapshot).expect("incomplete assertion should pass");
    assert_message_assembly_buffered_count(snapshot, 1)
        .expect("buffered-count assertion should pass");
    assert_message_assembly_total_buffered_bytes(snapshot, 5)
        .expect("total-buffered-bytes assertion should pass");
}

#[test]
fn message_assertion_helpers_accept_completed_bodies() {
    let message = assembled_message(7, b"payload");
    let last_result = Ok(Some(message.clone()));
    let completed = [message];
    let snapshot = MessageAssemblySnapshot::new(Some(&last_result), &completed, &[], 0, 0);

    assert_message_assembly_completed(snapshot, b"payload")
        .expect("completed-body assertion should pass");
    assert_message_assembly_completed_for_key(snapshot, MessageKey(7), b"payload")
        .expect("completed-for-key assertion should pass");
}

#[rstest]
#[case(
    MessageAssemblyError::Series(MessageSeriesError::SequenceMismatch {
        expected: FrameSequence(2),
        found: FrameSequence(3),
    }),
    MessageAssemblyErrorExpectation::SequenceMismatch {
        expected: FrameSequence(2),
        found: FrameSequence(3),
    }
)]
#[case(
    MessageAssemblyError::DuplicateFirstFrame {
        key: MessageKey(4),
    },
    MessageAssemblyErrorExpectation::DuplicateFirstFrame {
        key: MessageKey(4),
    }
)]
#[case(
    MessageAssemblyError::MessageTooLarge {
        key: MessageKey(9),
        attempted: 12,
        limit: std::num::NonZeroUsize::MIN,
    },
    MessageAssemblyErrorExpectation::MessageTooLarge {
        key: MessageKey(9),
    }
)]
#[case(
    MessageAssemblyError::Series(MessageSeriesError::MissingFirstFrame {
        key: MessageKey(11),
    }),
    MessageAssemblyErrorExpectation::MissingFirstFrame {
        key: MessageKey(11),
    }
)]
#[case(
    MessageAssemblyError::Series(MessageSeriesError::DuplicateFrame {
        key: MessageKey(5),
        sequence: FrameSequence(2),
    }),
    MessageAssemblyErrorExpectation::DuplicateFrame {
        key: MessageKey(5),
        sequence: FrameSequence(2),
    }
)]
#[case(
    MessageAssemblyError::ConnectionBudgetExceeded {
        key: MessageKey(6),
        attempted: 50,
        limit: std::num::NonZeroUsize::MIN,
    },
    MessageAssemblyErrorExpectation::ConnectionBudgetExceeded {
        key: MessageKey(6),
    }
)]
#[case(
    MessageAssemblyError::InFlightBudgetExceeded {
        key: MessageKey(8),
        attempted: 100,
        limit: std::num::NonZeroUsize::MIN,
    },
    MessageAssemblyErrorExpectation::InFlightBudgetExceeded {
        key: MessageKey(8),
    }
)]
fn message_assertion_helpers_match_errors(
    #[case] error: MessageAssemblyError,
    #[case] expected: MessageAssemblyErrorExpectation,
) {
    let last_result = Err(error);
    let snapshot = MessageAssemblySnapshot::new(Some(&last_result), &[], &[], 0, 0);

    assert_message_assembly_error(snapshot, expected)
        .expect("structured error assertion should pass");
}

#[test]
fn message_assertion_helpers_report_mismatch_details() {
    let last_result = Ok(None);
    let snapshot = MessageAssemblySnapshot::new(Some(&last_result), &[], &[], 0, 0);

    let err = assert_message_assembly_completed(snapshot, b"payload")
        .expect_err("completed assertion should fail for incomplete state");
    let message = err.to_string();

    assert!(
        message.contains("expected completed message assembly"),
        "unexpected error message: {message}"
    );
    assert!(
        message.contains("incomplete assembly"),
        "unexpected error message: {message}"
    );
}

#[test]
fn message_assertion_helpers_track_eviction() {
    let evicted = [MessageKey(12)];
    let snapshot = MessageAssemblySnapshot::new(None, &[], &evicted, 0, 0);

    assert_message_assembly_evicted(snapshot, MessageKey(12))
        .expect("eviction assertion should pass");
}

#[test]
fn fragment_assertion_helpers_accept_absent_completed_and_counts() {
    let message = ReassembledMessage::new(MessageId::new(21), vec![1, 2, 3, 4]);
    let empty = FragmentReassemblySnapshot::new(None, None, &[], 1);
    let completed = FragmentReassemblySnapshot::new(Some(&message), None, &[], 0);

    assert_fragment_reassembly_absent(empty).expect("absent assertion should pass");
    assert_fragment_reassembly_buffered_messages(empty, 1)
        .expect("buffered-count assertion should pass");
    assert_fragment_reassembly_completed_len(completed, 4)
        .expect("completed-length assertion should pass");
}

#[rstest]
#[case(
    wireframe::fragment::ReassemblyError::MessageTooLarge {
        message_id: MessageId::new(30),
        attempted: 10,
        limit: std::num::NonZeroUsize::MIN,
    },
    FragmentReassemblyErrorExpectation::MessageTooLarge {
        message_id: MessageId::new(30),
    }
)]
#[case(
    wireframe::fragment::ReassemblyError::Fragment(
        wireframe::fragment::FragmentError::IndexMismatch {
            expected: wireframe::fragment::FragmentIndex::new(1),
            found: wireframe::fragment::FragmentIndex::new(3),
        }
    ),
    FragmentReassemblyErrorExpectation::IndexMismatch
)]
fn fragment_assertion_helpers_match_errors(
    #[case] error: wireframe::fragment::ReassemblyError,
    #[case] expected: FragmentReassemblyErrorExpectation,
) {
    let snapshot = FragmentReassemblySnapshot::new(None, Some(&error), &[], 0);

    assert_fragment_reassembly_error(snapshot, expected)
        .expect("fragment error assertion should pass");
}

#[test]
fn fragment_assertion_helpers_track_eviction() {
    let evicted = [MessageId::new(23)];
    let snapshot = FragmentReassemblySnapshot::new(None, None, &evicted, 0);

    assert_fragment_reassembly_evicted(snapshot, MessageId::new(23))
        .expect("fragment eviction assertion should pass");
}

#[test]
fn fragment_error_assertion_fails_when_no_error_present() {
    let snapshot = FragmentReassemblySnapshot::new(None, None, &[], 0);

    let err = assert_fragment_reassembly_error(
        snapshot,
        FragmentReassemblyErrorExpectation::IndexMismatch,
    )
    .expect_err("error assertion should fail when no error is present");

    let message = err.to_string();
    assert!(
        message.contains("no reassembly error"),
        "diagnostic should mention missing error, got: {message}"
    );
}

#[test]
fn fragment_completed_len_fails_when_no_message_reassembled() {
    let snapshot = FragmentReassemblySnapshot::new(None, None, &[], 0);

    let err = assert_fragment_reassembly_completed_len(snapshot, 1)
        .expect_err("completed_len should fail when last_reassembled is None");

    let message = err.to_string();
    assert!(
        message.contains("no message reassembled"),
        "diagnostic should reference missing reassembled message, got: {message}"
    );
}
