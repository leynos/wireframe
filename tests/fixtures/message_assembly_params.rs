//! Parameter objects for message assembly test steps.

use wireframe::message_assembler::{FrameSequence, MessageKey};

/// Parameters for creating a first frame.
#[derive(Debug)]
pub struct FirstFrameParams {
    /// Message key.
    pub key: MessageKey,
    /// Metadata bytes.
    pub metadata: Vec<u8>,
    /// Body bytes.
    pub body: Vec<u8>,
    /// Whether this is the final frame.
    pub is_last: bool,
    /// Total body length declared for the logical message, if any.
    pub total_body_len: Option<usize>,
}

impl FirstFrameParams {
    /// Create parameters for a first frame with default values.
    #[must_use]
    pub fn new(key: MessageKey, body: Vec<u8>) -> Self {
        Self {
            key,
            metadata: vec![],
            body,
            is_last: false,
            total_body_len: None,
        }
    }

    /// Set metadata bytes.
    #[must_use]
    pub fn with_metadata(mut self, metadata: Vec<u8>) -> Self {
        self.metadata = metadata;
        self
    }

    /// Declare the total body length of the logical message.
    #[must_use]
    pub fn with_total(mut self, total_body_len: usize) -> Self {
        self.total_body_len = Some(total_body_len);
        self
    }

    /// Mark as the final frame.
    #[must_use]
    pub fn final_frame(mut self) -> Self {
        self.is_last = true;
        self
    }
}

/// Parameters for creating a continuation frame.
#[derive(Debug)]
pub struct ContinuationFrameParams {
    /// Message key.
    pub key: MessageKey,
    /// Optional sequence number.
    pub sequence: Option<FrameSequence>,
    /// Body bytes.
    pub body: Vec<u8>,
    /// Whether this is the final frame.
    pub is_last: bool,
}

impl ContinuationFrameParams {
    /// Create parameters for a continuation frame with default sequence 1.
    #[must_use]
    pub fn new(key: MessageKey, body: Vec<u8>) -> Self {
        Self {
            key,
            sequence: Some(FrameSequence(1)),
            body,
            is_last: false,
        }
    }

    /// Set the sequence number.
    #[must_use]
    pub fn with_sequence(mut self, sequence: FrameSequence) -> Self {
        self.sequence = Some(sequence);
        self
    }

    /// Mark as the final frame.
    #[must_use]
    pub fn final_frame(mut self) -> Self {
        self.is_last = true;
        self
    }
}
