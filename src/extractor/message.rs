//! Message extractor for deserialised payloads.

use super::{ExtractError, FromMessageRequest, MessageRequest, Payload};
use crate::message::Message as WireMessage;

/// Extractor that deserializes the message payload into `T`.
#[derive(Debug, Clone)]
pub struct Message<T>(T);

impl<T> Message<T> {
    /// Consumes the extractor and returns the inner deserialised message value.
    #[must_use]
    pub fn into_inner(self) -> T { self.0 }
}

impl<T> std::ops::Deref for Message<T> {
    type Target = T;

    /// Returns a reference to the inner value.
    ///
    /// This enables transparent access to the wrapped type via dereferencing.
    fn deref(&self) -> &Self::Target { &self.0 }
}

impl<T> FromMessageRequest for Message<T>
where
    T: WireMessage,
{
    type Error = ExtractError;

    /// Attempts to extract and deserialize a message of type `T` from the payload.
    ///
    /// Advances the payload by the number of bytes consumed during deserialization.
    /// Returns an error if the payload cannot be decoded into the target type.
    ///
    /// # Returns
    /// - `Ok(Self)`: The successfully extracted and deserialized message.
    /// - `Err(ExtractError::InvalidPayload)`: If deserialization fails.
    fn from_message_request(
        _req: &MessageRequest,
        payload: &mut Payload<'_>,
    ) -> Result<Self, Self::Error> {
        let (msg, consumed) = T::from_bytes(payload.data).map_err(ExtractError::InvalidPayload)?;
        payload.advance(consumed);
        Ok(Self(msg))
    }
}
