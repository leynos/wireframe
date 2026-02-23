//! Shared test serializer that captures deserialization context metadata.

use std::sync::{Arc, Mutex};

use wireframe::{
    app::Envelope,
    frame::FrameMetadata,
    message::{DecodeWith, DeserializeContext, EncodeWith},
    serializer::{BincodeSerializer, MessageCompatibilitySerializer, Serializer},
};

/// Captured metadata from [`DeserializeContext`].
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct CapturedDeserializeContext {
    /// Parsed message identifier from metadata, if available.
    pub message_id: Option<u32>,
    /// Parsed correlation identifier from metadata, if available.
    pub correlation_id: Option<u64>,
    /// Number of source bytes consumed while parsing metadata.
    pub metadata_bytes_consumed: Option<usize>,
    /// Raw frame metadata length, if available.
    pub frame_metadata_len: Option<usize>,
}

impl CapturedDeserializeContext {
    /// Build a captured snapshot from `context`.
    #[must_use]
    pub fn from_context(context: &DeserializeContext<'_>) -> Self {
        Self {
            message_id: context.message_id,
            correlation_id: context.correlation_id,
            metadata_bytes_consumed: context.metadata_bytes_consumed,
            frame_metadata_len: context.frame_metadata.map(<[u8]>::len),
        }
    }
}

/// Test serializer that stores the latest deserialization context.
#[derive(Default)]
pub struct ContextCapturingSerializer {
    captured: Arc<Mutex<Option<CapturedDeserializeContext>>>,
}

impl ContextCapturingSerializer {
    /// Construct a serializer that writes into `captured`.
    #[must_use]
    pub fn new(captured: Arc<Mutex<Option<CapturedDeserializeContext>>>) -> Self {
        Self { captured }
    }
}

impl MessageCompatibilitySerializer for ContextCapturingSerializer {}

impl Serializer for ContextCapturingSerializer {
    fn serialize<M>(&self, value: &M) -> Result<Vec<u8>, Box<dyn std::error::Error + Send + Sync>>
    where
        M: EncodeWith<Self>,
    {
        value.encode_with(self)
    }

    fn deserialize<M>(
        &self,
        bytes: &[u8],
    ) -> Result<(M, usize), Box<dyn std::error::Error + Send + Sync>>
    where
        M: DecodeWith<Self>,
    {
        M::decode_with(self, bytes, &DeserializeContext::empty())
    }

    fn deserialize_with_context<M>(
        &self,
        bytes: &[u8],
        context: &DeserializeContext<'_>,
    ) -> Result<(M, usize), Box<dyn std::error::Error + Send + Sync>>
    where
        M: DecodeWith<Self>,
    {
        let mut state = self.captured.lock().map_err(|_| {
            "ContextCapturingSerializer::deserialize_with_context captured mutex poisoned"
        })?;
        *state = Some(CapturedDeserializeContext::from_context(context));
        M::decode_with(self, bytes, context)
    }
}

impl FrameMetadata for ContextCapturingSerializer {
    type Frame = Envelope;
    type Error = bincode::error::DecodeError;

    fn parse(&self, src: &[u8]) -> Result<(Self::Frame, usize), Self::Error> {
        BincodeSerializer.parse(src)
    }
}
