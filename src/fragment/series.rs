//! Ordering tracker used to re-assemble logical messages from fragments.
//!
//! `FragmentSeries` is intentionally small so it can be embedded in codecs or
//! adapters without imposing allocation overhead.

use super::{FragmentError, FragmentHeader, FragmentIndex, FragmentStatus, MessageId};

/// Track the expected ordering of fragments for a single logical message.
///
/// The series keeps only lightweight metadata (current message id, expected
/// next index, completion flag) so it can be embedded inside the future
/// fragment adapter without additional allocations.
#[derive(Clone, Debug)]
pub struct FragmentSeries {
    message_id: MessageId,
    next_index: FragmentIndex,
    complete: bool,
}

impl FragmentSeries {
    /// Create a new series for `message_id`, expecting the first fragment.
    #[must_use]
    pub const fn new(message_id: MessageId) -> Self {
        Self {
            message_id,
            next_index: FragmentIndex::zero(),
            complete: false,
        }
    }

    /// Return the message identifier tracked by this series.
    #[must_use]
    pub const fn message_id(&self) -> MessageId { self.message_id }

    /// Return whether the series has consumed the final fragment.
    #[must_use]
    pub const fn is_complete(&self) -> bool { self.complete }

    /// Accept a fragment and update the expected index.
    ///
    /// # Examples
    ///
    /// ```
    /// use wireframe::fragment::{
    ///     FragmentHeader,
    ///     FragmentIndex,
    ///     FragmentSeries,
    ///     FragmentStatus,
    ///     MessageId,
    /// };
    /// let mut series = FragmentSeries::new(MessageId::new(99));
    /// let first = FragmentHeader::new(MessageId::new(99), FragmentIndex::zero(), false);
    /// let final_fragment = FragmentHeader::new(MessageId::new(99), FragmentIndex::new(1), true);
    /// assert_eq!(series.accept(first), Ok(FragmentStatus::Incomplete));
    /// assert_eq!(series.accept(final_fragment), Ok(FragmentStatus::Complete));
    /// assert!(series.is_complete());
    /// ```
    ///
    /// # Errors
    ///
    /// Returns [`FragmentError::MessageMismatch`] when the fragment belongs to
    /// a different message, [`FragmentError::IndexMismatch`] when the fragment
    /// arrives ahead of the expected index, [`FragmentError::SeriesComplete`]
    /// when the series already consumed a final fragment, and
    /// [`FragmentError::IndexOverflow`] when the fragment index cannot advance
    /// further.
    ///
    /// When a fragment repeats an already accepted index, this method returns
    /// [`FragmentStatus::Duplicate`] and leaves the series position unchanged.
    pub fn accept(&mut self, fragment: FragmentHeader) -> Result<FragmentStatus, FragmentError> {
        if fragment.message_id() != self.message_id {
            return Err(FragmentError::MessageMismatch {
                expected: self.message_id,
                found: fragment.message_id(),
            });
        }

        if self.complete {
            return Err(FragmentError::SeriesComplete);
        }

        if fragment.fragment_index() < self.next_index {
            return Ok(FragmentStatus::Duplicate);
        }

        if fragment.fragment_index() > self.next_index {
            return Err(FragmentError::IndexMismatch {
                expected: self.next_index,
                found: fragment.fragment_index(),
            });
        }

        let next_index = fragment.fragment_index().checked_increment();
        if fragment.is_last_fragment() {
            self.complete = true;
            if let Some(incremented) = next_index {
                self.next_index = incremented;
            }
            return Ok(FragmentStatus::Complete);
        }

        let Some(incremented) = next_index else {
            return Err(FragmentError::IndexOverflow {
                last: fragment.fragment_index(),
            });
        };

        self.next_index = incremented;
        Ok(FragmentStatus::Incomplete)
    }
}

impl FragmentSeries {
    /// Testing helper that forces the next expected fragment index.
    #[doc(hidden)]
    pub fn force_next_index_for_tests(&mut self, next: FragmentIndex) { self.next_index = next; }
}
