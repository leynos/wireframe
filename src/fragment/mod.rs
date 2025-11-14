//! Fragment metadata primitives for transparent message splitting.
//!
//! This module introduces domain types required by the forthcoming
//! fragmentation and re-assembly layer. The types are intentionally small and
//! copyable so that codecs, connection actors, and behavioural tests can share
//! them without incurring allocation overhead. They serve as the canonical
//! representation of per-fragment metadata across the stack.

use std::{fmt, num::TryFromIntError};

use bincode::{Decode, Encode};
use thiserror::Error;

/// Unique identifier for a logical message undergoing fragmentation.
///
/// # Examples
///
/// ```
/// use wireframe::fragment::MessageId;
/// let id = MessageId::new(42);
/// assert_eq!(id.get(), 42);
/// ```
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Encode, Decode)]
pub struct MessageId(u64);

impl MessageId {
    /// Create a new identifier.
    #[must_use]
    pub const fn new(value: u64) -> Self { Self(value) }

    /// Return the inner numeric identifier.
    #[must_use]
    pub const fn get(self) -> u64 { self.0 }
}

impl From<u64> for MessageId {
    fn from(value: u64) -> Self { Self(value) }
}

impl From<MessageId> for u64 {
    fn from(value: MessageId) -> Self { value.0 }
}

impl fmt::Display for MessageId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result { write!(f, "{}", self.0) }
}

/// Zero-based ordinal describing a fragment's position within its message.
///
/// # Examples
///
/// ```
/// use wireframe::fragment::FragmentIndex;
/// let index = FragmentIndex::new(3);
/// assert_eq!(index.get(), 3);
/// assert!(index.checked_increment().is_some());
/// ```
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Encode, Decode)]
pub struct FragmentIndex(u32);

impl FragmentIndex {
    /// Construct an index from a `u32` value.
    #[must_use]
    pub const fn new(value: u32) -> Self { Self(value) }

    /// Return the first valid fragment index.
    #[must_use]
    pub const fn zero() -> Self { Self(0) }

    /// Return the underlying numeric value.
    #[must_use]
    pub const fn get(self) -> u32 { self.0 }

    /// Increment the index, returning `None` on overflow.
    #[must_use]
    pub const fn checked_increment(self) -> Option<Self> {
        match self.0.checked_add(1) {
            Some(next) => Some(Self(next)),
            None => None,
        }
    }
}

impl TryFrom<usize> for FragmentIndex {
    type Error = TryFromIntError;

    fn try_from(value: usize) -> Result<Self, Self::Error> { u32::try_from(value).map(Self) }
}

impl From<u32> for FragmentIndex {
    fn from(value: u32) -> Self { Self(value) }
}

impl From<FragmentIndex> for u32 {
    fn from(value: FragmentIndex) -> Self { value.0 }
}

impl fmt::Display for FragmentIndex {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result { write!(f, "{}", self.0) }
}

/// Header describing a single fragment.
///
/// `FragmentHeader` is agnostic of the underlying frame or serializer. It
/// captures just enough information for codecs to stitch fragments back
/// together while remaining small enough to copy by value.
///
/// # Examples
///
/// ```
/// use wireframe::fragment::{FragmentHeader, FragmentIndex, MessageId};
/// let header = FragmentHeader::new(MessageId::new(7), FragmentIndex::zero(), false);
/// assert_eq!(header.message_id().get(), 7);
/// assert_eq!(header.fragment_index().get(), 0);
/// assert!(!header.is_last_fragment());
/// ```
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Encode, Decode)]
pub struct FragmentHeader {
    message_id: MessageId,
    fragment_index: FragmentIndex,
    is_last_fragment: bool,
}

impl FragmentHeader {
    /// Create a new fragment header.
    #[must_use]
    pub const fn new(
        message_id: MessageId,
        fragment_index: FragmentIndex,
        is_last_fragment: bool,
    ) -> Self {
        Self {
            message_id,
            fragment_index,
            is_last_fragment,
        }
    }

    /// Return the logical message identifier.
    #[must_use]
    pub const fn message_id(&self) -> MessageId { self.message_id }

    /// Return the fragment position relative to the message.
    #[must_use]
    pub const fn fragment_index(&self) -> FragmentIndex { self.fragment_index }

    /// Report whether this is the final fragment.
    #[must_use]
    pub const fn is_last_fragment(&self) -> bool { self.is_last_fragment }
}

/// Result of feeding a fragment into a [`FragmentSeries`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FragmentStatus {
    /// The logical message still expects more fragments.
    Incomplete,
    /// The fragment completed the logical message.
    Complete,
}

/// Errors produced by [`FragmentSeries`].
#[derive(Clone, Copy, Debug, Error, PartialEq, Eq)]
pub enum FragmentError {
    /// The fragment belongs to a different message.
    #[error("fragment message mismatch: expected {expected}, found {found}")]
    MessageMismatch {
        expected: MessageId,
        found: MessageId,
    },
    /// A fragment arrived out of order.
    #[error("fragment index mismatch: expected {expected}, found {found}")]
    IndexMismatch {
        expected: FragmentIndex,
        found: FragmentIndex,
    },
    /// The series already consumed a last fragment.
    #[error("fragment series already complete")]
    SeriesComplete,
    /// The fragment index overflowed `u32::MAX`.
    #[error("fragment index overflow after {last}")]
    IndexOverflow { last: FragmentIndex },
}

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
    /// arrives out of order, [`FragmentError::SeriesComplete`] when the series
    /// already consumed a final fragment, and [`FragmentError::IndexOverflow`]
    /// when the fragment index cannot advance further.
    pub fn accept(&mut self, fragment: FragmentHeader) -> Result<FragmentStatus, FragmentError> {
        if fragment.message_id != self.message_id {
            return Err(FragmentError::MessageMismatch {
                expected: self.message_id,
                found: fragment.message_id,
            });
        }

        if self.complete {
            return Err(FragmentError::SeriesComplete);
        }

        if fragment.fragment_index != self.next_index {
            return Err(FragmentError::IndexMismatch {
                expected: self.next_index,
                found: fragment.fragment_index,
            });
        }

        let next_index =
            fragment
                .fragment_index
                .checked_increment()
                .ok_or(FragmentError::IndexOverflow {
                    last: fragment.fragment_index,
                })?;

        self.next_index = next_index;
        if fragment.is_last_fragment {
            self.complete = true;
            Ok(FragmentStatus::Complete)
        } else {
            Ok(FragmentStatus::Incomplete)
        }
    }
}

#[cfg(test)]
mod tests {
    use rstest::rstest;

    use super::*;

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
        // Prime the series so it expects the maximum index next.
        series.next_index = FragmentIndex::new(u32::MAX);
        let header = FragmentHeader::new(MessageId::new(1), FragmentIndex::new(u32::MAX), false);
        let err = series
            .accept(header)
            .expect_err("overflow must raise an error");
        assert!(matches!(err, FragmentError::IndexOverflow { .. }));
    }
}
