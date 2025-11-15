use thiserror::Error;

use super::{FragmentIndex, MessageId};

/// Result of feeding a fragment into a [`FragmentSeries`](crate::fragment::FragmentSeries).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FragmentStatus {
    /// The logical message still expects more fragments.
    Incomplete,
    /// The fragment completed the logical message.
    Complete,
}

/// Errors produced by [`FragmentSeries`](crate::fragment::FragmentSeries).
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
