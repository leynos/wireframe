//! Assertion helpers for transport fragment reassembly outcomes.

use super::assert_helpers::{assert_body_eq, assert_usize_field};
use crate::{
    fragment::{FragmentError, MessageId, ReassembledMessage, ReassemblyError},
    testkit::TestResult,
};

/// Snapshot of the observable state around a fragment-reassembly assertion.
#[derive(Clone, Copy, Debug)]
pub struct FragmentReassemblySnapshot<'a> {
    last_reassembled: Option<&'a ReassembledMessage>,
    last_error: Option<&'a ReassemblyError>,
    evicted_ids: &'a [MessageId],
    buffered_messages: usize,
}

impl<'a> FragmentReassemblySnapshot<'a> {
    /// Create a snapshot from the caller's current reassembly state.
    #[must_use]
    pub fn new(
        last_reassembled: Option<&'a ReassembledMessage>,
        last_error: Option<&'a ReassemblyError>,
        evicted_ids: &'a [MessageId],
        buffered_messages: usize,
    ) -> Self {
        Self {
            last_reassembled,
            last_error,
            evicted_ids,
            buffered_messages,
        }
    }
}

/// Expected fragment-reassembly error shape.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FragmentReassemblyErrorExpectation {
    /// Expect an over-limit assembled message for a specific identifier.
    MessageTooLarge {
        /// Message identifier that exceeded the cap.
        message_id: MessageId,
    },
    /// Expect an over-limit assembled message regardless of identifier.
    MessageTooLargeAny,
    /// Expect an out-of-order fragment.
    IndexMismatch,
    /// Expect a fragment from the wrong logical message.
    MessageMismatch,
    /// Expect a duplicate-or-late fragment after completion.
    SeriesComplete,
    /// Expect fragment-index overflow.
    IndexOverflow,
}

/// Assert that no message has been reassembled yet.
///
/// # Errors
///
/// Returns an error if a reassembled payload is present.
pub fn assert_fragment_reassembly_absent(snapshot: FragmentReassemblySnapshot<'_>) -> TestResult {
    if snapshot.last_reassembled.is_none() {
        Ok(())
    } else {
        Err("unexpected reassembled message present".into())
    }
}

/// Assert that the last reassembled payload length equals `expected_len`.
///
/// # Errors
///
/// Returns an error if no message was reassembled or the length differs.
pub fn assert_fragment_reassembly_completed_len(
    snapshot: FragmentReassemblySnapshot<'_>,
    expected_len: usize,
) -> TestResult {
    let Some(message) = snapshot.last_reassembled else {
        return Err("no message reassembled".into());
    };
    assert_usize_field(message.payload().len(), expected_len, "payload length")
}

/// Assert that the last reassembled payload bytes match `expected`.
///
/// # Errors
///
/// Returns an error if no message was reassembled or the payload differs.
pub fn assert_fragment_reassembly_completed_bytes(
    snapshot: FragmentReassemblySnapshot<'_>,
    expected: &[u8],
) -> TestResult {
    let Some(message) = snapshot.last_reassembled else {
        return Err("no message reassembled".into());
    };
    assert_body_eq(message.payload(), expected, "reassembled payload")
}

/// Assert that the last reassembly error matches `expected`.
///
/// # Errors
///
/// Returns an error if no error was captured or the error does not match.
pub fn assert_fragment_reassembly_error(
    snapshot: FragmentReassemblySnapshot<'_>,
    expected: FragmentReassemblyErrorExpectation,
) -> TestResult {
    let Some(err) = snapshot.last_error else {
        return Err("no reassembly error captured".into());
    };
    if matches_fragment_error(err, expected) {
        Ok(())
    } else {
        Err(format!("expected {expected:?}, got {err:?}").into())
    }
}

/// Assert that `expected` partial messages are still buffered.
///
/// # Errors
///
/// Returns an error if the buffered-message count differs.
pub fn assert_fragment_reassembly_buffered_messages(
    snapshot: FragmentReassemblySnapshot<'_>,
    expected: usize,
) -> TestResult {
    assert_usize_field(snapshot.buffered_messages, expected, "buffered messages")
}

/// Assert that `message_id` was evicted during the most recent purge.
///
/// # Errors
///
/// Returns an error if the identifier is not in the eviction list.
pub fn assert_fragment_reassembly_evicted(
    snapshot: FragmentReassemblySnapshot<'_>,
    message_id: MessageId,
) -> TestResult {
    if snapshot.evicted_ids.contains(&message_id) {
        Ok(())
    } else {
        Err(format!("message {} was not evicted", message_id.get()).into())
    }
}

fn matches_fragment_error(
    err: &ReassemblyError,
    expected: FragmentReassemblyErrorExpectation,
) -> bool {
    match expected {
        FragmentReassemblyErrorExpectation::MessageTooLarge { message_id } => {
            matches!(
                err,
                ReassemblyError::MessageTooLarge {
                    message_id: actual_id,
                    ..
                } if *actual_id == message_id
            )
        }
        FragmentReassemblyErrorExpectation::MessageTooLargeAny => {
            matches!(err, ReassemblyError::MessageTooLarge { .. })
        }
        FragmentReassemblyErrorExpectation::IndexMismatch => matches!(
            err,
            ReassemblyError::Fragment(FragmentError::IndexMismatch { .. })
        ),
        FragmentReassemblyErrorExpectation::MessageMismatch => matches!(
            err,
            ReassemblyError::Fragment(FragmentError::MessageMismatch { .. })
        ),
        FragmentReassemblyErrorExpectation::SeriesComplete => matches!(
            err,
            ReassemblyError::Fragment(FragmentError::SeriesComplete)
        ),
        FragmentReassemblyErrorExpectation::IndexOverflow => matches!(
            err,
            ReassemblyError::Fragment(FragmentError::IndexOverflow { .. })
        ),
    }
}
