//! Error and status types emitted by the fragmentation layer.
//!
//! These enums keep both the outbound and inbound logic decoupled from
//! specific protocols while still surfacing precise diagnostics for
//! behavioural tests.

use std::num::NonZeroUsize;

use bincode::error::EncodeError;
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

/// Errors produced while fragmenting outbound messages.
#[derive(Debug, Error)]
pub enum FragmentationError {
    /// Serialisation failed before chunking.
    #[error("failed to encode message: {0}")]
    Encode(#[from] EncodeError),
    /// The fragment index cannot advance because it would overflow `u32`.
    #[error("fragment index overflow after {last}")]
    IndexOverflow { last: FragmentIndex },
}

/// Errors produced while re-assembling inbound fragments.
#[derive(Clone, Copy, Debug, Error, PartialEq, Eq)]
pub enum ReassemblyError {
    /// The fragment broke ordering or message tracking guarantees.
    #[error("fragment rejected during reassembly: {0}")]
    Fragment(#[from] FragmentError),
    /// The combined fragment payloads would exceed the configured cap.
    #[error("message {message_id} exceeds reassembly cap: {attempted} bytes > {limit} byte limit")]
    MessageTooLarge {
        /// Identifier for the logical message being assembled.
        message_id: MessageId,
        /// Total size that triggered the guard.
        attempted: usize,
        /// Configured reassembly cap.
        limit: NonZeroUsize,
    },
}
