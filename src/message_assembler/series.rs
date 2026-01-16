//! Ordering tracker for protocol-level message frame series.
//!
//! `MessageSeries` tracks the expected ordering of frames belonging to a
//! single logical message. It is intentionally lightweight so it can be
//! embedded in per-message assembly state without imposing allocation overhead.

use super::{
    ContinuationFrameHeader,
    FirstFrameHeader,
    FrameSequence,
    MessageKey,
    error::{MessageSeriesError, MessageSeriesStatus},
};

/// Determines how sequence numbers are tracked for a message series.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum SequenceTracking {
    /// Protocol does not supply sequence numbers; ordering is not validated.
    Untracked,
    /// Protocol supplies sequence numbers; enforce strict ordering.
    Tracked,
}

/// Track the expected ordering of frames for a single message key.
///
/// The series keeps lightweight metadata (message key, expected next sequence,
/// completion flag) so it can be embedded in per-message assembly state.
///
/// # Examples
///
/// ```
/// use wireframe::message_assembler::{
///     ContinuationFrameHeader,
///     FirstFrameHeader,
///     FrameSequence,
///     MessageKey,
///     MessageSeries,
///     MessageSeriesStatus,
/// };
///
/// let first = FirstFrameHeader {
///     message_key: MessageKey(1),
///     metadata_len: 0,
///     body_len: 10,
///     total_body_len: None,
///     is_last: false,
/// };
/// let mut series = MessageSeries::from_first_frame(&first);
///
/// let cont = ContinuationFrameHeader {
///     message_key: MessageKey(1),
///     sequence: Some(FrameSequence(1)),
///     body_len: 5,
///     is_last: true,
/// };
/// assert_eq!(
///     series.accept_continuation(&cont),
///     Ok(MessageSeriesStatus::Complete)
/// );
/// assert!(series.is_complete());
/// ```
#[derive(Clone, Debug)]
pub struct MessageSeries {
    message_key: MessageKey,
    /// Next expected sequence number (None if protocol does not supply them).
    next_sequence: Option<FrameSequence>,
    /// Whether sequences are being enforced for this series.
    sequence_tracking: SequenceTracking,
    /// Whether the series has received a final frame.
    complete: bool,
    /// Expected total body length, if declared in the first frame.
    expected_total: Option<usize>,
}

impl MessageSeries {
    /// Create a new series from a first frame header.
    ///
    /// # Examples
    ///
    /// ```
    /// use wireframe::message_assembler::{FirstFrameHeader, MessageKey, MessageSeries};
    ///
    /// let header = FirstFrameHeader {
    ///     message_key: MessageKey(42),
    ///     metadata_len: 0,
    ///     body_len: 100,
    ///     total_body_len: Some(500),
    ///     is_last: false,
    /// };
    /// let series = MessageSeries::from_first_frame(&header);
    /// assert_eq!(series.message_key(), MessageKey(42));
    /// assert_eq!(series.expected_total(), Some(500));
    /// assert!(!series.is_complete());
    /// ```
    #[must_use]
    pub fn from_first_frame(header: &FirstFrameHeader) -> Self {
        Self {
            message_key: header.message_key,
            next_sequence: None,
            sequence_tracking: SequenceTracking::Untracked,
            expected_total: header.total_body_len,
            complete: header.is_last,
        }
    }

    /// Return the message key tracked by this series.
    #[must_use]
    pub const fn message_key(&self) -> MessageKey { self.message_key }

    /// Return whether the series has consumed the final frame.
    #[must_use]
    pub const fn is_complete(&self) -> bool { self.complete }

    /// Return the expected total body length, if declared.
    #[must_use]
    pub const fn expected_total(&self) -> Option<usize> { self.expected_total }

    /// Accept a continuation frame and update the expected sequence.
    ///
    /// # Errors
    ///
    /// Returns [`MessageSeriesError::KeyMismatch`] when the frame belongs to a
    /// different message, [`MessageSeriesError::SequenceMismatch`] when the
    /// frame arrives out of order, [`MessageSeriesError::SeriesComplete`] when
    /// the series already consumed a final frame,
    /// [`MessageSeriesError::SequenceOverflow`] when the sequence cannot
    /// advance further, and [`MessageSeriesError::DuplicateFrame`] when a
    /// sequence number has already been processed.
    ///
    /// # Examples
    ///
    /// ```
    /// use wireframe::message_assembler::{
    ///     ContinuationFrameHeader,
    ///     FirstFrameHeader,
    ///     FrameSequence,
    ///     MessageKey,
    ///     MessageSeries,
    ///     MessageSeriesError,
    ///     MessageSeriesStatus,
    /// };
    ///
    /// let first = FirstFrameHeader {
    ///     message_key: MessageKey(1),
    ///     metadata_len: 0,
    ///     body_len: 10,
    ///     total_body_len: None,
    ///     is_last: false,
    /// };
    /// let mut series = MessageSeries::from_first_frame(&first);
    ///
    /// // Continuation with wrong key is rejected
    /// let wrong_key = ContinuationFrameHeader {
    ///     message_key: MessageKey(2),
    ///     sequence: Some(FrameSequence(1)),
    ///     body_len: 5,
    ///     is_last: false,
    /// };
    /// assert!(matches!(
    ///     series.accept_continuation(&wrong_key),
    ///     Err(MessageSeriesError::KeyMismatch { .. })
    /// ));
    /// ```
    pub fn accept_continuation(
        &mut self,
        header: &ContinuationFrameHeader,
    ) -> Result<MessageSeriesStatus, MessageSeriesError> {
        // Validate message key
        if header.message_key != self.message_key {
            return Err(MessageSeriesError::KeyMismatch {
                expected: self.message_key,
                found: header.message_key,
            });
        }

        // Reject if already complete
        if self.complete {
            return Err(MessageSeriesError::SeriesComplete);
        }

        // Validate sequence ordering if protocol supplies sequences
        if let Some(incoming_seq) = header.sequence {
            self.validate_and_advance_sequence(incoming_seq, header.is_last)?;
        } else if self.sequence_tracking == SequenceTracking::Tracked {
            // Once tracking is active, reject frames without sequence numbers
            return Err(MessageSeriesError::MissingSequence {
                key: self.message_key,
            });
        }

        // Mark complete if this is the final frame
        if header.is_last {
            self.complete = true;
            return Ok(MessageSeriesStatus::Complete);
        }

        Ok(MessageSeriesStatus::Incomplete)
    }

    fn validate_and_advance_sequence(
        &mut self,
        incoming: FrameSequence,
        is_last: bool,
    ) -> Result<(), MessageSeriesError> {
        match self.sequence_tracking {
            SequenceTracking::Untracked => {
                // First continuation with a sequence number; start tracking
                self.sequence_tracking = SequenceTracking::Tracked;
                self.next_sequence = incoming.checked_increment();
                // Overflow is only an error if more frames are expected
                if self.next_sequence.is_none() && !is_last {
                    return Err(MessageSeriesError::SequenceOverflow { last: incoming });
                }
                Ok(())
            }
            SequenceTracking::Tracked => {
                let expected = self
                    .next_sequence
                    .ok_or(MessageSeriesError::SequenceOverflow { last: incoming })?;

                if incoming.0 < expected.0 {
                    // Duplicate: sequence already seen
                    return Err(MessageSeriesError::DuplicateFrame {
                        key: self.message_key,
                        sequence: incoming,
                    });
                }

                if incoming != expected {
                    // Out of order or gap
                    return Err(MessageSeriesError::SequenceMismatch {
                        expected,
                        found: incoming,
                    });
                }

                // Advance expected sequence
                self.next_sequence = incoming.checked_increment();
                // Overflow is only an error if more frames are expected
                if self.next_sequence.is_none() && !is_last {
                    return Err(MessageSeriesError::SequenceOverflow { last: incoming });
                }
                Ok(())
            }
        }
    }
}

impl MessageSeries {
    /// Testing helper that forces the next expected sequence.
    #[doc(hidden)]
    pub fn force_next_sequence_for_tests(&mut self, next: FrameSequence) {
        self.sequence_tracking = SequenceTracking::Tracked;
        self.next_sequence = Some(next);
    }
}
