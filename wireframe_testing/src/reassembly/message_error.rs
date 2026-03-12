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
            matches!(
                err,
                MessageAssemblyError::Series(MessageSeriesError::SequenceMismatch {
                    expected: actual_expected,
                    found: actual_found,
                }) if *actual_expected == expected && *actual_found == found
            )
        }
        MessageAssemblyErrorExpectation::DuplicateFrame { key, sequence } => {
            matches!(
                err,
                MessageAssemblyError::Series(MessageSeriesError::DuplicateFrame {
                    key: actual_key,
                    sequence: actual_sequence,
                }) if *actual_key == key && *actual_sequence == sequence
            )
        }
        MessageAssemblyErrorExpectation::MissingFirstFrame { key } => {
            matches!(
                err,
                MessageAssemblyError::Series(MessageSeriesError::MissingFirstFrame {
                    key: actual_key,
                }) if *actual_key == key
            )
        }
        MessageAssemblyErrorExpectation::DuplicateFirstFrame { key } => {
            matches!(
                err,
                MessageAssemblyError::DuplicateFirstFrame { key: actual_key }
                    if *actual_key == key
            )
        }
        MessageAssemblyErrorExpectation::MessageTooLarge { key } => {
            matches!(
                err,
                MessageAssemblyError::MessageTooLarge { key: actual_key, .. }
                    if *actual_key == key
            )
        }
        MessageAssemblyErrorExpectation::ConnectionBudgetExceeded { key } => {
            matches!(
                err,
                MessageAssemblyError::ConnectionBudgetExceeded { key: actual_key, .. }
                    if *actual_key == key
            )
        }
        MessageAssemblyErrorExpectation::InFlightBudgetExceeded { key } => {
            matches!(
                err,
                MessageAssemblyError::InFlightBudgetExceeded { key: actual_key, .. }
                    if *actual_key == key
            )
        }
    }
}
