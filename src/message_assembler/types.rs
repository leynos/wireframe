//! Public types for message assembly inputs and outputs.
//!
//! This module contains `FirstFrameInput`, `FirstFrameInputError`, and
//! `AssembledMessage`, extracted from `state.rs` to meet the 400-line file
//! limit.

use thiserror::Error;

use super::{FirstFrameHeader, MessageKey};

/// Input data for a first frame.
///
/// Groups the header and payload components that comprise a first frame.
///
/// # Examples
///
/// ```
/// use wireframe::message_assembler::{FirstFrameHeader, FirstFrameInput, MessageKey};
///
/// let header = FirstFrameHeader {
///     message_key: MessageKey(1),
///     metadata_len: 2,
///     body_len: 5,
///     total_body_len: None,
///     is_last: false,
/// };
/// let input = FirstFrameInput::new(&header, vec![0x01, 0x02], b"hello")
///     .expect("header lengths match payload sizes");
/// assert_eq!(input.header.message_key, MessageKey(1));
/// ```
#[derive(Debug)]
pub struct FirstFrameInput<'a> {
    /// The frame header.
    pub header: &'a FirstFrameHeader,
    /// Protocol-specific metadata.
    pub metadata: Vec<u8>,
    /// Body payload slice.
    pub body: &'a [u8],
}

/// Error returned when [`FirstFrameInput`] validation fails.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Error)]
pub enum FirstFrameInputError {
    /// Metadata length in header does not match actual metadata size.
    #[error("metadata length mismatch: header declares {header_len} bytes, got {actual_len}")]
    MetadataLengthMismatch {
        /// Length declared in header.
        header_len: usize,
        /// Actual length of metadata slice.
        actual_len: usize,
    },
    /// Body length in header does not match actual body size.
    #[error("body length mismatch: header declares {header_len} bytes, got {actual_len}")]
    BodyLengthMismatch {
        /// Length declared in header.
        header_len: usize,
        /// Actual length of body slice.
        actual_len: usize,
    },
}

impl<'a> FirstFrameInput<'a> {
    /// Create a new first frame input, validating header lengths against payloads.
    ///
    /// # Errors
    ///
    /// Returns an error if `header.metadata_len` does not match `metadata.len()`
    /// or `header.body_len` does not match `body.len()`.
    pub fn new(
        header: &'a FirstFrameHeader,
        metadata: Vec<u8>,
        body: &'a [u8],
    ) -> Result<Self, FirstFrameInputError> {
        if header.metadata_len != metadata.len() {
            return Err(FirstFrameInputError::MetadataLengthMismatch {
                header_len: header.metadata_len,
                actual_len: metadata.len(),
            });
        }
        if header.body_len != body.len() {
            return Err(FirstFrameInputError::BodyLengthMismatch {
                header_len: header.body_len,
                actual_len: body.len(),
            });
        }
        Ok(Self {
            header,
            metadata,
            body,
        })
    }
}

/// Container for a fully assembled message.
///
/// # Examples
///
/// ```
/// use wireframe::message_assembler::{AssembledMessage, MessageKey};
///
/// // Normally obtained from MessageAssemblyState::accept_first_frame or
/// // accept_continuation_frame when a message completes.
/// let msg = AssembledMessage::new(MessageKey(1), vec![0x01], vec![0x02, 0x03]);
/// assert_eq!(msg.message_key(), MessageKey(1));
/// assert_eq!(msg.metadata(), &[0x01]);
/// assert_eq!(msg.body(), &[0x02, 0x03]);
/// ```
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AssembledMessage {
    message_key: MessageKey,
    metadata: Vec<u8>,
    body: Vec<u8>,
}

impl AssembledMessage {
    /// Create a new assembled message.
    #[must_use]
    pub fn new(message_key: MessageKey, metadata: Vec<u8>, body: Vec<u8>) -> Self {
        Self {
            message_key,
            metadata,
            body,
        }
    }

    /// Message key that correlated the frames.
    #[must_use]
    pub const fn message_key(&self) -> MessageKey { self.message_key }

    /// Protocol-specific metadata from the first frame.
    #[must_use]
    pub fn metadata(&self) -> &[u8] { &self.metadata }

    /// Reassembled body bytes.
    #[must_use]
    pub fn body(&self) -> &[u8] { &self.body }

    /// Consume and return the body bytes.
    #[must_use]
    pub fn into_body(self) -> Vec<u8> { self.body }
}
