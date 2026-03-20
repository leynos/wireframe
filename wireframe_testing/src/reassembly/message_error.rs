//! Shared message-assembly expectations and matching logic.

use wireframe::message_assembler::{
    FrameSequence,
    MessageAssemblyError,
    MessageKey,
    MessageSeriesError,
};

/// Expected message-assembly error shape.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MessageAssemblyErrorExpectation {
    /// Expect a sequence mismatch.
    SequenceMismatch {
        /// Sequence the series expected next.
        expected: FrameSequence,
        /// Sequence carried by the received frame.
        found: FrameSequence,
    },
    /// Expect a duplicate continuation frame.
    DuplicateFrame {
        /// Message key the duplicate belongs to.
        key: MessageKey,
        /// Sequence number that was duplicated.
        sequence: FrameSequence,
    },
    /// Expect a missing-first-frame error.
    MissingFirstFrame {
        /// Message key that has no in-progress assembly.
        key: MessageKey,
    },
    /// Expect a duplicate first frame.
    DuplicateFirstFrame {
        /// Message key that already had an in-progress assembly.
        key: MessageKey,
    },
    /// Expect an over-limit assembled message.
    MessageTooLarge {
        /// Message key that exceeded the size cap.
        key: MessageKey,
    },
    /// Expect a connection-budget breach.
    ConnectionBudgetExceeded {
        /// Message key whose frame triggered the budget guard.
        key: MessageKey,
    },
    /// Expect an in-flight budget breach.
    InFlightBudgetExceeded {
        /// Message key whose frame triggered the budget guard.
        key: MessageKey,
    },
}

pub(super) fn matches_message_error(
    err: &MessageAssemblyError,
    expected: MessageAssemblyErrorExpectation,
) -> bool {
    match expected {
        MessageAssemblyErrorExpectation::SequenceMismatch { expected, found } => {
            matches_sequence_mismatch(err, expected, found)
        }
        MessageAssemblyErrorExpectation::DuplicateFrame { key, sequence } => {
            matches_duplicate_frame(err, key, sequence)
        }
        MessageAssemblyErrorExpectation::MissingFirstFrame { key } => {
            matches_missing_first_frame(err, key)
        }
        MessageAssemblyErrorExpectation::DuplicateFirstFrame { key } => {
            matches_duplicate_first_frame(err, key)
        }
        MessageAssemblyErrorExpectation::MessageTooLarge { key } => {
            matches_message_too_large(err, key)
        }
        MessageAssemblyErrorExpectation::ConnectionBudgetExceeded { key } => {
            matches_connection_budget_exceeded(err, key)
        }
        MessageAssemblyErrorExpectation::InFlightBudgetExceeded { key } => {
            matches_in_flight_budget_exceeded(err, key)
        }
    }
}

fn matches_sequence_mismatch(
    err: &MessageAssemblyError,
    expected: FrameSequence,
    found: FrameSequence,
) -> bool {
    matches!(
        err,
        MessageAssemblyError::Series(MessageSeriesError::SequenceMismatch {
            expected: actual_expected,
            found: actual_found,
        }) if *actual_expected == expected && *actual_found == found
    )
}

fn matches_duplicate_frame(
    err: &MessageAssemblyError,
    key: MessageKey,
    sequence: FrameSequence,
) -> bool {
    matches!(
        err,
        MessageAssemblyError::Series(MessageSeriesError::DuplicateFrame {
            key: actual_key,
            sequence: actual_sequence,
        }) if *actual_key == key && *actual_sequence == sequence
    )
}

fn matches_missing_first_frame(err: &MessageAssemblyError, key: MessageKey) -> bool {
    matches!(
        err,
        MessageAssemblyError::Series(MessageSeriesError::MissingFirstFrame {
            key: actual_key,
        }) if *actual_key == key
    )
}

fn matches_duplicate_first_frame(err: &MessageAssemblyError, key: MessageKey) -> bool {
    matches!(
        err,
        MessageAssemblyError::DuplicateFirstFrame { key: actual_key }
            if *actual_key == key
    )
}

fn matches_message_too_large(err: &MessageAssemblyError, key: MessageKey) -> bool {
    matches!(
        err,
        MessageAssemblyError::MessageTooLarge { key: actual_key, .. }
            if *actual_key == key
    )
}

fn matches_connection_budget_exceeded(err: &MessageAssemblyError, key: MessageKey) -> bool {
    matches!(
        err,
        MessageAssemblyError::ConnectionBudgetExceeded { key: actual_key, .. }
            if *actual_key == key
    )
}

fn matches_in_flight_budget_exceeded(err: &MessageAssemblyError, key: MessageKey) -> bool {
    matches!(
        err,
        MessageAssemblyError::InFlightBudgetExceeded { key: actual_key, .. }
            if *actual_key == key
    )
}
