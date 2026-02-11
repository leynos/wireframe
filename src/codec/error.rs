//! Error types for the codec layer.
//!
//! This module provides a structured error taxonomy that distinguishes between
//! framing errors (wire-level frame boundary issues), protocol errors (semantic
//! violations after frame extraction), I/O errors, and EOF conditions.
//!
//! # Error Categories
//!
//! - [`FramingError`]: Wire-level issues in frame structure (oversized frames, invalid length
//!   encoding, incomplete headers).
//! - [`ProtocolError`]: Higher-level protocol violations (missing headers, unsupported versions,
//!   sequence violations).
//! - [`EofError`]: End-of-stream conditions distinguishing clean closure from premature
//!   disconnection.
//! - [`CodecError`]: Top-level enum wrapping all categories plus I/O errors.
//!
//! # Recovery Policies
//!
//! Each error type has a default recovery policy accessible via
//! [`CodecError::default_recovery_policy`]:
//!
//! - [`RecoveryPolicy::Drop`]: Discard the malformed frame and continue.
//! - [`RecoveryPolicy::Quarantine`]: Pause the connection temporarily.
//! - [`RecoveryPolicy::Disconnect`]: Terminate the connection.

use std::io;

use thiserror::Error;

use super::recovery::RecoveryPolicy;

/// Framing-level errors occurring during frame boundary detection.
///
/// These errors indicate problems with the wire-level frame structure,
/// typically occurring before any payload interpretation.
#[derive(Clone, Debug, Error, PartialEq, Eq)]
pub enum FramingError {
    /// Frame length prefix indicates size exceeding configured maximum.
    #[error("frame exceeds max length: {size} > {max}")]
    OversizedFrame {
        /// Actual frame size indicated by the length prefix.
        size: usize,
        /// Maximum allowed frame size.
        max: usize,
    },

    /// Frame length prefix is malformed or corrupt.
    #[error("invalid frame length encoding")]
    InvalidLengthEncoding,

    /// Incomplete frame header received (need more bytes).
    #[error("incomplete frame header: have {have}, need {need}")]
    IncompleteHeader {
        /// Bytes currently available.
        have: usize,
        /// Bytes required for complete header.
        need: usize,
    },

    /// Frame checksum mismatch (for protocols using checksums).
    #[error("frame checksum mismatch: expected {expected:#x}, got {actual:#x}")]
    ChecksumMismatch {
        /// Expected checksum value.
        expected: u32,
        /// Actual checksum computed from frame data.
        actual: u32,
    },

    /// Zero-length frame received where non-empty is required.
    #[error("empty frame not permitted")]
    EmptyFrame,
}

/// Protocol-level errors occurring after successful frame extraction.
///
/// These errors indicate semantic violations in the protocol layer,
/// after the frame boundaries have been successfully determined.
#[derive(Clone, Debug, Error, PartialEq, Eq)]
pub enum ProtocolError {
    /// Required protocol header field is missing or malformed.
    #[error("missing required header field: {field}")]
    MissingHeader {
        /// Name of the missing or malformed field.
        field: String,
    },

    /// Protocol version mismatch or unsupported version.
    #[error("unsupported protocol version: {version}")]
    UnsupportedVersion {
        /// Version number that was rejected.
        version: u32,
    },

    /// Invalid message type identifier.
    #[error("unknown message type: {type_id}")]
    UnknownMessageType {
        /// Message type identifier that was not recognised.
        type_id: u32,
    },

    /// Message sequence violation (duplicate or out-of-order).
    #[error("sequence violation: expected {expected}, got {actual}")]
    SequenceViolation {
        /// Expected sequence number.
        expected: u64,
        /// Actual sequence number received.
        actual: u64,
    },

    /// Protocol state machine violation.
    #[error("invalid state transition: {from} -> {to}")]
    InvalidStateTransition {
        /// State the protocol was in.
        from: String,
        /// State that was incorrectly attempted.
        to: String,
    },
}

/// EOF handling variants distinguishing normal vs. premature closure.
///
/// These errors help differentiate between a clean connection close
/// (at a frame boundary) and a premature disconnection (mid-frame).
#[derive(Clone, Copy, Debug, Error, PartialEq, Eq)]
pub enum EofError {
    /// Clean EOF at frame boundary - normal socket closure.
    ///
    /// This indicates the peer closed the connection gracefully after
    /// completing the last frame. No data was lost.
    #[error("connection closed cleanly at frame boundary")]
    CleanClose,

    /// EOF received mid-frame - premature socket closure.
    ///
    /// The peer closed the connection while a frame was being read.
    /// Some data may have been lost.
    #[error("premature EOF: {bytes_received} bytes of {expected} byte frame received")]
    MidFrame {
        /// Bytes received before EOF.
        bytes_received: usize,
        /// Expected total frame size (if known).
        expected: usize,
    },

    /// EOF received mid-header during length prefix read.
    ///
    /// The peer closed the connection while the frame header was being read.
    #[error("premature EOF during header: {bytes_received} of {header_size} header bytes")]
    MidHeader {
        /// Header bytes received before EOF.
        bytes_received: usize,
        /// Expected header size.
        header_size: usize,
    },
}

/// Top-level codec error taxonomy.
///
/// This enum provides a unified error type for all codec-layer failures,
/// categorised by their origin and recovery semantics.
///
/// # Examples
///
/// ```
/// use wireframe::codec::{CodecError, FramingError, RecoveryPolicy};
///
/// let err = CodecError::Framing(FramingError::OversizedFrame {
///     size: 2000,
///     max: 1024,
/// });
///
/// assert_eq!(err.default_recovery_policy(), RecoveryPolicy::Drop);
/// assert!(!err.should_disconnect());
/// ```
#[derive(Debug, Error)]
pub enum CodecError {
    /// Framing layer error (wire-level frame boundary issues).
    #[error("framing error: {0}")]
    Framing(#[from] FramingError),

    /// Protocol layer error (post-frame extraction issues).
    #[error("protocol error: {0}")]
    Protocol(#[from] ProtocolError),

    /// Transport layer I/O error.
    #[error("I/O error: {0}")]
    Io(#[from] io::Error),

    /// End-of-stream handling.
    #[error("EOF: {0}")]
    Eof(#[from] EofError),
}

impl CodecError {
    /// Returns the recommended recovery policy for this error.
    ///
    /// # Default Policies
    ///
    /// | Error Type | Policy |
    /// |------------|--------|
    /// | `Framing::OversizedFrame` | `Drop` |
    /// | `Framing::EmptyFrame` | `Drop` |
    /// | Other `Framing` errors | `Disconnect` |
    /// | All `Protocol` errors | `Drop` |
    /// | All `Io` errors | `Disconnect` |
    /// | `Eof::CleanClose` | `Disconnect` (graceful) |
    /// | Other `Eof` errors | `Disconnect` |
    ///
    /// # Examples
    ///
    /// ```
    /// use wireframe::codec::{CodecError, FramingError, RecoveryPolicy};
    ///
    /// let err = CodecError::Framing(FramingError::OversizedFrame {
    ///     size: 2000,
    ///     max: 1024,
    /// });
    /// assert_eq!(err.default_recovery_policy(), RecoveryPolicy::Drop);
    ///
    /// let err = CodecError::Io(std::io::Error::other("connection reset"));
    /// assert_eq!(err.default_recovery_policy(), RecoveryPolicy::Disconnect);
    /// ```
    #[must_use]
    pub fn default_recovery_policy(&self) -> RecoveryPolicy {
        match self {
            // Recoverable errors: drop and continue
            Self::Framing(FramingError::OversizedFrame { .. } | FramingError::EmptyFrame)
            | Self::Protocol(_) => RecoveryPolicy::Drop,
            // Unrecoverable errors: connection must be terminated
            Self::Framing(_) | Self::Io(_) | Self::Eof(_) => RecoveryPolicy::Disconnect,
        }
    }

    /// Returns true if this error represents a clean connection close.
    ///
    /// A clean close occurs when the peer closes the connection at a frame
    /// boundary, indicating no data was lost.
    ///
    /// # Examples
    ///
    /// ```
    /// use wireframe::codec::{CodecError, EofError};
    ///
    /// let err = CodecError::Eof(EofError::CleanClose);
    /// assert!(err.is_clean_close());
    ///
    /// let err = CodecError::Eof(EofError::MidFrame {
    ///     bytes_received: 100,
    ///     expected: 200,
    /// });
    /// assert!(!err.is_clean_close());
    /// ```
    #[must_use]
    pub fn is_clean_close(&self) -> bool { matches!(self, Self::Eof(EofError::CleanClose)) }

    /// Returns true if the connection should be terminated.
    ///
    /// This is a convenience method that checks whether the default recovery
    /// policy is [`RecoveryPolicy::Disconnect`].
    ///
    /// # Examples
    ///
    /// ```
    /// use wireframe::codec::{CodecError, FramingError};
    ///
    /// // Oversized frames can be dropped without disconnecting
    /// let err = CodecError::Framing(FramingError::OversizedFrame {
    ///     size: 2000,
    ///     max: 1024,
    /// });
    /// assert!(!err.should_disconnect());
    ///
    /// // Invalid length encoding corrupts framing state
    /// let err = CodecError::Framing(FramingError::InvalidLengthEncoding);
    /// assert!(err.should_disconnect());
    /// ```
    #[must_use]
    pub fn should_disconnect(&self) -> bool {
        self.default_recovery_policy() == RecoveryPolicy::Disconnect
    }

    /// Returns the error category as a string for logging and metrics.
    ///
    /// # Returns
    ///
    /// One of: `"framing"`, `"protocol"`, `"io"`, or `"eof"`.
    #[must_use]
    pub fn error_type(&self) -> &'static str {
        match self {
            Self::Framing(_) => "framing",
            Self::Protocol(_) => "protocol",
            Self::Io(_) => "io",
            Self::Eof(_) => "eof",
        }
    }
}

impl From<CodecError> for io::Error {
    fn from(err: CodecError) -> Self {
        match err {
            CodecError::Io(e) => e,
            CodecError::Framing(e) => io::Error::new(io::ErrorKind::InvalidData, e),
            CodecError::Protocol(e) => io::Error::new(io::ErrorKind::InvalidData, e),
            CodecError::Eof(e) => io::Error::new(io::ErrorKind::UnexpectedEof, e),
        }
    }
}

#[cfg(test)]
#[path = "error_tests.rs"]
mod tests;
