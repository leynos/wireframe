//! Message trait and serialization helpers.
//!
//! Types implementing [`Message`] can be encoded and decoded using
//! `bincode` with standard configuration.

use std::error::Error;

use bincode::{
    BorrowDecode,
    Encode,
    borrow_decode_from_slice,
    config,
    encode_to_vec,
    error::{DecodeError, EncodeError},
};

/// Context supplied to metadata-aware deserializers.
///
/// This structure carries the metadata fields extracted during
/// [`crate::frame::FrameMetadata::parse`] so serializers can select versioned
/// decode paths without depending on hidden global state.
#[derive(Clone, Copy, Debug, Default)]
pub struct DeserializeContext<'a> {
    /// Raw metadata bytes extracted from the frame, if available.
    pub frame_metadata: Option<&'a [u8]>,
    /// Parsed message identifier from metadata, if available.
    pub message_id: Option<u32>,
    /// Parsed correlation identifier from metadata, if available.
    pub correlation_id: Option<u64>,
    /// Number of source bytes consumed while parsing metadata.
    pub metadata_bytes_consumed: Option<usize>,
}

impl DeserializeContext<'_> {
    /// Return an empty context.
    #[must_use]
    pub const fn empty() -> Self {
        Self {
            frame_metadata: None,
            message_id: None,
            correlation_id: None,
            metadata_bytes_consumed: None,
        }
    }
}

/// Serializer-agnostic encoding adapter used by [`crate::serializer::Serializer`].
pub trait EncodeWith<S: ?Sized> {
    /// Encode `self` with `serializer`.
    ///
    /// # Errors
    ///
    /// Returns an error when encoding fails.
    fn encode_with(&self, serializer: &S) -> Result<Vec<u8>, Box<dyn Error + Send + Sync>>;
}

/// Serializer-agnostic decoding adapter used by [`crate::serializer::Serializer`].
pub trait DecodeWith<S: ?Sized>: Sized {
    /// Decode `Self` from `bytes` with optional metadata context.
    ///
    /// # Errors
    ///
    /// Returns an error when decoding fails.
    fn decode_with(
        serializer: &S,
        bytes: &[u8],
        context: &DeserializeContext<'_>,
    ) -> Result<(Self, usize), Box<dyn Error + Send + Sync>>;
}

/// Wrapper trait for application message types.
///
/// Any type deriving [`Encode`] and [`BorrowDecode`] automatically implements
/// this trait via a blanket implementation. The default methods provide
/// convenient helpers to serialize and deserialize using bincode's
/// standard configuration.
pub trait Message: Encode + for<'de> BorrowDecode<'de, ()> {
    /// Serialize the message into a byte vector.
    ///
    /// # Errors
    ///
    /// Returns an [`EncodeError`] if serialization fails.
    fn to_bytes(&self) -> Result<Vec<u8>, EncodeError> { encode_to_vec(self, config::standard()) }

    /// Deserialize a message from a byte slice, returning the message and
    /// the number of bytes consumed.
    ///
    /// # Errors
    ///
    /// Deserializes a message instance from a byte slice using the standard configuration.
    ///
    /// Returns the deserialized message and the number of bytes consumed, or a [`DecodeError`] if
    /// deserialization fails.
    ///
    /// # Examples
    ///
    /// ```no_run
    /// use wireframe::message::Message;
    /// #[derive(bincode::Encode, bincode::BorrowDecode)]
    /// struct MyMessageType(u8);
    /// let bytes = vec![]; // serialized message bytes
    /// let (_msg, consumed) =
    ///     MyMessageType::from_bytes(&bytes).expect("Failed to decode message bytes");
    /// assert!(consumed <= bytes.len());
    /// ```
    fn from_bytes(bytes: &[u8]) -> Result<(Self, usize), DecodeError>
    where
        Self: Sized,
    {
        borrow_decode_from_slice(bytes, config::standard())
    }
}

impl<T> Message for T where T: Encode + for<'de> BorrowDecode<'de, ()> {}

impl<S, T> EncodeWith<S> for T
where
    S: crate::serializer::MessageCompatibilitySerializer + ?Sized,
    T: Message,
{
    fn encode_with(&self, _serializer: &S) -> Result<Vec<u8>, Box<dyn Error + Send + Sync>> {
        self.to_bytes()
            .map_err(|error| Box::new(error) as Box<dyn Error + Send + Sync>)
    }
}

impl<S, T> DecodeWith<S> for T
where
    S: crate::serializer::MessageCompatibilitySerializer + ?Sized,
    T: Message,
{
    fn decode_with(
        _serializer: &S,
        bytes: &[u8],
        _context: &DeserializeContext<'_>,
    ) -> Result<(Self, usize), Box<dyn Error + Send + Sync>> {
        T::from_bytes(bytes).map_err(|error| Box::new(error) as Box<dyn Error + Send + Sync>)
    }
}

#[cfg(feature = "serializer-serde")]
pub mod serde_bridge {
    //! Optional Serde wrapper adapters for serializer bridges.

    use serde::{Serialize, de::DeserializeOwned};

    use super::{DecodeWith, DeserializeContext, EncodeWith};
    use crate::serializer::SerdeSerializerBridge;

    /// Wrapper providing `EncodeWith`/`DecodeWith` implementations via Serde.
    #[derive(Clone, Debug, PartialEq, Eq)]
    pub struct SerdeMessage<T>(T);

    impl<T> SerdeMessage<T> {
        /// Wrap a value for Serde bridge encoding.
        #[must_use]
        pub const fn new(value: T) -> Self { Self(value) }

        /// Borrow the wrapped value.
        #[must_use]
        pub const fn as_ref(&self) -> &T { &self.0 }

        /// Consume the wrapper and return the inner value.
        #[must_use]
        pub fn into_inner(self) -> T { self.0 }
    }

    impl<T> From<T> for SerdeMessage<T> {
        fn from(value: T) -> Self { Self(value) }
    }

    impl<S, T> EncodeWith<S> for SerdeMessage<T>
    where
        S: SerdeSerializerBridge + ?Sized,
        T: Serialize,
    {
        fn encode_with(
            &self,
            serializer: &S,
        ) -> Result<Vec<u8>, Box<dyn std::error::Error + Send + Sync>> {
            serializer.serialize_serde(self.as_ref())
        }
    }

    impl<S, T> DecodeWith<S> for SerdeMessage<T>
    where
        S: SerdeSerializerBridge + ?Sized,
        T: DeserializeOwned,
    {
        fn decode_with(
            serializer: &S,
            bytes: &[u8],
            context: &DeserializeContext<'_>,
        ) -> Result<(Self, usize), Box<dyn std::error::Error + Send + Sync>> {
            let (decoded, consumed) = serializer.deserialize_serde(bytes, context)?;
            Ok((Self::new(decoded), consumed))
        }
    }

    /// Helper trait to wrap values ergonomically for Serde bridge calls.
    pub trait IntoSerdeMessage: Sized {
        /// Wrap `self` as a [`SerdeMessage`].
        fn into_serde_message(self) -> SerdeMessage<Self> { SerdeMessage::new(self) }
    }

    impl<T> IntoSerdeMessage for T {}
}

#[cfg(test)]
mod tests {
    use rstest::rstest;

    use super::{DecodeWith, DeserializeContext, EncodeWith};
    use crate::serializer::BincodeSerializer;

    #[derive(bincode::Decode, bincode::Encode, Debug, PartialEq, Eq)]
    struct RoundTripMessage {
        value: u32,
    }

    #[rstest]
    fn bincode_adapter_round_trip() {
        let serializer = BincodeSerializer;
        let original = RoundTripMessage { value: 41 };

        let bytes = original
            .encode_with(&serializer)
            .expect("bincode adapter should encode message");
        let (decoded, consumed) =
            RoundTripMessage::decode_with(&serializer, &bytes, &DeserializeContext::empty())
                .expect("bincode adapter should decode encoded message");

        assert_eq!(decoded, original);
        assert_eq!(consumed, bytes.len());
    }

    #[rstest]
    fn deserialize_context_empty_defaults_to_none() {
        let context = DeserializeContext::empty();
        assert!(context.frame_metadata.is_none());
        assert!(context.message_id.is_none());
        assert!(context.correlation_id.is_none());
        assert!(context.metadata_bytes_consumed.is_none());
    }

    #[cfg(feature = "serializer-serde")]
    mod serde_bridge_tests {
        use rstest::rstest;
        use serde::{Deserialize, Serialize};

        use super::super::{
            DecodeWith,
            DeserializeContext,
            EncodeWith,
            serde_bridge::{IntoSerdeMessage, SerdeMessage},
        };
        use crate::serializer::BincodeSerializer;

        #[derive(Debug, PartialEq, Eq, Serialize, Deserialize)]
        struct SerdePayload {
            id: u32,
        }

        #[rstest]
        fn serde_message_round_trip_with_bincode_bridge() {
            let serializer = BincodeSerializer;
            let original = SerdePayload { id: 9 }.into_serde_message();

            let bytes = original
                .encode_with(&serializer)
                .expect("serde bridge should encode payload");
            let (decoded, consumed) = SerdeMessage::<SerdePayload>::decode_with(
                &serializer,
                &bytes,
                &DeserializeContext::empty(),
            )
            .expect("serde bridge should decode encoded payload");

            assert_eq!(decoded.into_inner().id, 9);
            assert_eq!(consumed, bytes.len());
        }
    }
}
