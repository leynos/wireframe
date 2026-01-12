//! Error and status types for protocol-level message assembly.
//!
//! These enums keep the message assembly logic decoupled from specific
//! protocols while surfacing precise diagnostics for behavioural tests.

use std::num::NonZeroUsize;

use thiserror::Error;

use super::{FrameSequence, MessageKey};

/// Result of feeding a frame into a [`MessageSeries`](super::MessageSeries).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MessageSeriesStatus {
    /// The message still expects more frames.
    Incomplete,
    /// The frame completed the message.
    Complete,
}

/// Errors produced by [`MessageSeries`](super::MessageSeries).
#[derive(Clone, Copy, Debug, Error, PartialEq, Eq)]
pub enum MessageSeriesError {
    /// The frame belongs to a different message.
    #[error("message key mismatch: expected {expected}, found {found}")]
    KeyMismatch {
        /// Message key currently being assembled.
        expected: MessageKey,
        /// Key carried by the incoming frame.
        found: MessageKey,
    },
    /// A frame arrived out of order (sequence number mismatch).
    #[error("frame sequence mismatch: expected {expected}, found {found}")]
    SequenceMismatch {
        /// Sequence the series expected next.
        expected: FrameSequence,
        /// Sequence carried by the frame that was received.
        found: FrameSequence,
    },
    /// The series already consumed a final frame.
    #[error("message series already complete")]
    SeriesComplete,
    /// The frame sequence overflowed `u32::MAX`.
    #[error("frame sequence overflow after {last}")]
    SequenceOverflow {
        /// Last valid sequence observed before overflow occurred.
        last: FrameSequence,
    },
    /// A continuation frame arrived before any first frame for this key.
    #[error("continuation frame received without first frame for key {key}")]
    MissingFirstFrame {
        /// Message key that has no in-progress assembly.
        key: MessageKey,
    },
    /// Duplicate frame detected (same sequence number already processed).
    #[error("duplicate frame: sequence {sequence} already received for key {key}")]
    DuplicateFrame {
        /// Message key the duplicate belongs to.
        key: MessageKey,
        /// Sequence number that was duplicated.
        sequence: FrameSequence,
    },
    /// A continuation frame arrived without a sequence number after sequence
    /// tracking was activated.
    #[error("continuation frame missing sequence number for key {key}")]
    MissingSequence {
        /// Message key that expected a sequence number.
        key: MessageKey,
    },
}

/// Errors produced during message assembly.
#[derive(Clone, Debug, Error, PartialEq, Eq)]
pub enum MessageAssemblyError {
    /// A continuity or ordering error from the series tracker.
    #[error("series validation failed: {0}")]
    Series(#[from] MessageSeriesError),

    /// A first frame arrived for a key that already has an in-progress
    /// assembly.
    #[error("duplicate first frame for key {key}")]
    DuplicateFirstFrame {
        /// Message key that already has an active assembly.
        key: MessageKey,
    },

    /// The assembled message would exceed the configured size limit.
    #[error("message {key} exceeds size limit: {attempted} bytes > {limit} bytes")]
    MessageTooLarge {
        /// Message key that exceeded the limit.
        key: MessageKey,
        /// Total size that triggered the guard.
        attempted: usize,
        /// Configured size cap.
        limit: NonZeroUsize,
    },
}
