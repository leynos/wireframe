//! Assertion helpers for transport fragment reassembly outcomes.

use wireframe::fragment::{FragmentError, MessageId, ReassembledMessage, ReassemblyError};

use crate::integration_helpers::TestResult;

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
    ///
    /// # Examples
    ///
    /// ```rust,no_run
    /// use wireframe_testing::reassembly::FragmentReassemblySnapshot;
    ///
    /// let snapshot = FragmentReassemblySnapshot::new(None, None, &[], 0);
    /// let _ = snapshot;
    /// ```
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
    /// Expect an over-limit assembled message.
    MessageTooLarge {
        /// Message identifier that exceeded the cap.
        message_id: MessageId,
    },
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
///
/// # Examples
///
/// ```rust,no_run
/// use wireframe_testing::reassembly::{
///     FragmentReassemblySnapshot,
///     assert_fragment_reassembly_absent,
/// };
///
/// let snapshot = FragmentReassemblySnapshot::new(None, None, &[], 0);
/// assert_fragment_reassembly_absent(snapshot)?;
/// # Ok::<(), wireframe_testing::TestError>(())
/// ```
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
///
/// # Examples
///
/// ```rust,no_run
/// use wireframe_testing::reassembly::{
///     FragmentReassemblySnapshot,
///     assert_fragment_reassembly_completed_len,
/// };
///
/// # fn run(snapshot: FragmentReassemblySnapshot<'_>) -> wireframe_testing::TestResult {
/// assert_fragment_reassembly_completed_len(snapshot, 7)?;
/// # Ok(())
/// # }
/// ```
pub fn assert_fragment_reassembly_completed_len(
    snapshot: FragmentReassemblySnapshot<'_>,
    expected_len: usize,
) -> TestResult {
    let Some(message) = snapshot.last_reassembled else {
        return Err("no message reassembled".into());
    };
    let actual = message.payload().len();
    if actual == expected_len {
        Ok(())
    } else {
        Err(format!("payload length mismatch: expected {expected_len}, got {actual}").into())
    }
}

/// Assert that the last reassembly error matches `expected`.
///
/// # Errors
///
/// Returns an error if no error was captured or the error does not match.
///
/// # Examples
///
/// ```rust,no_run
/// use wireframe::fragment::MessageId;
/// use wireframe_testing::reassembly::{
///     FragmentReassemblyErrorExpectation,
///     FragmentReassemblySnapshot,
///     assert_fragment_reassembly_error,
/// };
///
/// # fn run(snapshot: FragmentReassemblySnapshot<'_>) -> wireframe_testing::TestResult {
/// assert_fragment_reassembly_error(
///     snapshot,
///     FragmentReassemblyErrorExpectation::MessageTooLarge {
///         message_id: MessageId::new(7),
///     },
/// )?;
/// # Ok(())
/// # }
/// ```
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
///
/// # Examples
///
/// ```rust,no_run
/// use wireframe_testing::reassembly::{
///     FragmentReassemblySnapshot,
///     assert_fragment_reassembly_buffered_messages,
/// };
///
/// let snapshot = FragmentReassemblySnapshot::new(None, None, &[], 1);
/// assert_fragment_reassembly_buffered_messages(snapshot, 1)?;
/// # Ok::<(), wireframe_testing::TestError>(())
/// ```
pub fn assert_fragment_reassembly_buffered_messages(
    snapshot: FragmentReassemblySnapshot<'_>,
    expected: usize,
) -> TestResult {
    if snapshot.buffered_messages == expected {
        Ok(())
    } else {
        Err(format!(
            "expected {expected} buffered messages, got {}",
            snapshot.buffered_messages
        )
        .into())
    }
}

/// Assert that `message_id` was evicted during the most recent purge.
///
/// # Errors
///
/// Returns an error if the identifier is not in the eviction list.
///
/// # Examples
///
/// ```rust,no_run
/// use wireframe::fragment::MessageId;
/// use wireframe_testing::reassembly::{
///     FragmentReassemblySnapshot,
///     assert_fragment_reassembly_evicted,
/// };
///
/// let evicted = [MessageId::new(23)];
/// let snapshot = FragmentReassemblySnapshot::new(None, None, &evicted, 0);
/// assert_fragment_reassembly_evicted(snapshot, MessageId::new(23))?;
/// # Ok::<(), wireframe_testing::TestError>(())
/// ```
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
