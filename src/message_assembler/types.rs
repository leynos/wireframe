//! Public types for message assembly inputs and outputs.
//!
//! This module contains `FirstFrameInput`, `FirstFrameInputError`,
//! `EnvelopeRouting`, and `AssembledMessage`, extracted from `state.rs`
//! to meet the 400-line file limit.

use thiserror::Error;

use super::{FirstFrameHeader, MessageKey};

/// Routing metadata from the transport envelope that carried a first frame.
///
/// Captured at first-frame time so the completed [`AssembledMessage`] can
/// be dispatched to the correct handler and logged under the original
/// correlation identifier, regardless of which continuation frame
/// completed the assembly.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct EnvelopeRouting {
    /// Envelope identifier from the enclosing transport frame.
    pub envelope_id: u32,
    /// Correlation identifier from the enclosing transport frame.
    pub correlation_id: Option<u64>,
}

/// Input data for a first frame.
///
/// Groups the header and payload components that comprise a first frame,
/// along with the [`EnvelopeRouting`] metadata so the assembler can
/// preserve it for the completed message.
///
/// # Examples
///
/// ```
/// use wireframe::message_assembler::{
///     EnvelopeRouting,
///     FirstFrameHeader,
///     FirstFrameInput,
///     MessageKey,
/// };
///
/// let header = FirstFrameHeader {
///     message_key: MessageKey(1),
///     metadata_len: 2,
///     body_len: 5,
///     total_body_len: None,
///     is_last: false,
/// };
/// let routing = EnvelopeRouting {
///     envelope_id: 42,
///     correlation_id: Some(7),
/// };
/// let input = FirstFrameInput::new(&header, routing, vec![0x01, 0x02], b"hello")
///     .expect("header lengths match payload sizes");
/// assert_eq!(input.header.message_key, MessageKey(1));
/// assert_eq!(input.routing.envelope_id, 42);
/// ```
#[derive(Debug)]
pub struct FirstFrameInput<'a> {
    /// The frame header.
    pub header: &'a FirstFrameHeader,
    /// Envelope routing metadata from the enclosing transport frame.
    pub routing: EnvelopeRouting,
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
        routing: EnvelopeRouting,
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
            routing,
            metadata,
            body,
        })
    }
}

/// Container for a fully assembled message.
///
/// Preserves [`EnvelopeRouting`] from the first frame so that completed
/// messages are dispatched to the correct handler and logged under the
/// original correlation identifier.
///
/// # Examples
///
/// ```
/// use wireframe::message_assembler::{AssembledMessage, EnvelopeRouting, MessageKey};
///
/// // Normally obtained from MessageAssemblyState::accept_first_frame or
/// // accept_continuation_frame when a message completes.
/// let routing = EnvelopeRouting {
///     envelope_id: 42,
///     correlation_id: Some(7),
/// };
/// let msg = AssembledMessage::new(MessageKey(1), routing, vec![0x01], vec![0x02, 0x03]);
/// assert_eq!(msg.message_key(), MessageKey(1));
/// assert_eq!(msg.routing().envelope_id, 42);
/// assert_eq!(msg.routing().correlation_id, Some(7));
/// assert_eq!(msg.metadata(), &[0x01]);
/// assert_eq!(msg.body(), &[0x02, 0x03]);
/// ```
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AssembledMessage {
    message_key: MessageKey,
    routing: EnvelopeRouting,
    metadata: Vec<u8>,
    body: Vec<u8>,
}

impl AssembledMessage {
    /// Create a new assembled message.
    #[must_use]
    pub fn new(
        message_key: MessageKey,
        routing: EnvelopeRouting,
        metadata: Vec<u8>,
        body: Vec<u8>,
    ) -> Self {
        Self {
            message_key,
            routing,
            metadata,
            body,
        }
    }

    /// Message key that correlated the frames.
    #[must_use]
    pub const fn message_key(&self) -> MessageKey { self.message_key }

    /// Envelope routing metadata from the first frame.
    #[must_use]
    pub const fn routing(&self) -> EnvelopeRouting { self.routing }

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
