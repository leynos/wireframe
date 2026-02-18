//! Newtype wrappers and `StreamTestEnvelope` for streaming BDD tests.

use bytes::Bytes;
use derive_more::{Display, From};
use wireframe::{
    BincodeSerializer,
    Serializer,
    app::{Packet, PacketParts},
    correlation::CorrelatableFrame,
};

/// A correlation identifier used to match streaming requests to responses.
#[derive(
    Clone, Copy, Debug, Display, From, PartialEq, Eq, bincode::BorrowDecode, bincode::Encode,
)]
#[display("{_0}")]
pub(crate) struct CorrelationId(u64);

impl CorrelationId {
    pub(crate) const fn new(value: u64) -> Self { Self(value) }
    pub(crate) const fn get(self) -> u64 { self.0 }
}

/// A message identifier used for routing.
#[derive(
    Clone, Copy, Debug, Display, From, PartialEq, Eq, bincode::BorrowDecode, bincode::Encode,
)]
#[display("{_0}")]
pub(crate) struct MessageId(u32);

impl MessageId {
    pub(crate) const fn new(value: u32) -> Self { Self(value) }
    pub(crate) const fn get(self) -> u32 { self.0 }
}

/// Payload bytes carried by a test frame.
#[derive(Clone, Debug, PartialEq, Eq, bincode::BorrowDecode, bincode::Encode)]
pub(crate) struct Payload(Vec<u8>);

impl Payload {
    pub(crate) fn new(data: Vec<u8>) -> Self { Self(data) }
    pub(crate) fn into_inner(self) -> Vec<u8> { self.0 }
}

/// Terminator message ID used by the test streaming protocol.
pub(crate) const TERMINATOR_ID: MessageId = MessageId::new(0);

/// A test envelope that treats `id == 0` as a stream terminator.
#[derive(bincode::BorrowDecode, bincode::Encode, Debug, Clone, PartialEq, Eq)]
pub struct StreamTestEnvelope {
    pub id: MessageId,
    pub correlation_id: Option<CorrelationId>,
    pub payload: Payload,
}

impl CorrelatableFrame for StreamTestEnvelope {
    fn correlation_id(&self) -> Option<u64> { self.correlation_id.map(CorrelationId::get) }
    fn set_correlation_id(&mut self, cid: Option<u64>) {
        self.correlation_id = cid.map(CorrelationId::new);
    }
}

impl Packet for StreamTestEnvelope {
    fn id(&self) -> u32 { self.id.get() }
    fn into_parts(self) -> PacketParts {
        PacketParts::new(
            self.id.get(),
            self.correlation_id.map(CorrelationId::get),
            self.payload.into_inner(),
        )
    }
    fn from_parts(parts: PacketParts) -> Self {
        Self {
            id: MessageId::new(parts.id()),
            correlation_id: parts.correlation_id().map(CorrelationId::new),
            payload: Payload::new(parts.into_payload()),
        }
    }
    fn is_stream_terminator(&self) -> bool { self.id == TERMINATOR_ID }
}

impl StreamTestEnvelope {
    pub(crate) fn data(id: MessageId, correlation_id: CorrelationId, payload: Payload) -> Self {
        Self {
            id,
            correlation_id: Some(correlation_id),
            payload,
        }
    }
    pub(crate) fn terminator(correlation_id: CorrelationId) -> Self {
        Self {
            id: TERMINATOR_ID,
            correlation_id: Some(correlation_id),
            payload: Payload::new(vec![]),
        }
    }

    /// Serialize this envelope to bytes for transmission.
    ///
    /// # Errors
    /// Returns an error if serialization fails.
    pub(crate) fn serialize_to_bytes(
        &self,
    ) -> Result<Bytes, Box<dyn std::error::Error + Send + Sync>> {
        Ok(Bytes::from(BincodeSerializer.serialize(self)?))
    }
}
