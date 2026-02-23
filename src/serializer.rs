//! Message serialization traits.
//!
//! This module defines the [`Serializer`] trait enabling applications to plug in
//! custom encoding formats. A basic [`BincodeSerializer`] implementation is
//! provided as the default.

use std::error::Error;

#[cfg(feature = "serializer-serde")]
use bincode::config;
#[cfg(feature = "serializer-serde")]
use bincode::serde::{decode_from_slice, encode_to_vec};

use crate::{
    frame::FrameMetadata,
    message::{DecodeWith, DeserializeContext, EncodeWith, Message},
};

/// Marker trait for serializers that opt into legacy [`Message`] compatibility.
///
/// Implement this trait when `T: Message` values should automatically satisfy
/// `EncodeWith<Self>` and `DecodeWith<Self>`.
pub trait MessageCompatibilitySerializer {}

/// Optional bridge trait used by `message::serde_bridge`.
#[cfg(feature = "serializer-serde")]
pub trait SerdeSerializerBridge {
    /// Serialize a Serde value into bytes.
    ///
    /// # Errors
    ///
    /// Returns an error if serialization fails.
    fn serialize_serde<T: serde::Serialize>(
        &self,
        value: &T,
    ) -> Result<Vec<u8>, Box<dyn Error + Send + Sync>>;

    /// Deserialize a Serde value from bytes with optional metadata context.
    ///
    /// # Errors
    ///
    /// Returns an error if deserialization fails.
    fn deserialize_serde<T: serde::de::DeserializeOwned>(
        &self,
        bytes: &[u8],
        context: &DeserializeContext<'_>,
    ) -> Result<(T, usize), Box<dyn Error + Send + Sync>>;
}

/// Trait for serializing and deserializing messages.
///
/// # Object Safety
///
/// This trait is not object-safe. Its core methods include `Self: Sized`
/// bounds, so `dyn Serializer` cannot call `serialize`, `deserialize`, or
/// `deserialize_with_context`.
///
/// Use concrete serializer types (for example `BincodeSerializer`) in API
/// bounds. If runtime selection is required, introduce an explicit type-erased
/// wrapper that provides object-safe forwarding methods.
pub trait Serializer {
    /// Serialize `value` into a byte vector.
    ///
    /// # Errors
    ///
    /// Returns an error if the value cannot be serialized.
    fn serialize<M>(&self, value: &M) -> Result<Vec<u8>, Box<dyn Error + Send + Sync>>
    where
        M: EncodeWith<Self>,
        Self: Sized;

    /// Deserialize a message from `bytes`, returning the message and bytes consumed.
    ///
    /// # Errors
    ///
    /// Returns an error if the bytes cannot be parsed into a message.
    fn deserialize<M>(&self, bytes: &[u8]) -> Result<(M, usize), Box<dyn Error + Send + Sync>>
    where
        M: DecodeWith<Self>,
        Self: Sized;

    /// Deserialize a message from `bytes` using parsed frame metadata context.
    ///
    /// The default implementation delegates to [`Serializer::deserialize`],
    /// preserving existing serializers that are not metadata-aware.
    ///
    /// # Errors
    ///
    /// Returns an error if the bytes cannot be parsed into a message.
    fn deserialize_with_context<M>(
        &self,
        bytes: &[u8],
        _context: &DeserializeContext<'_>,
    ) -> Result<(M, usize), Box<dyn Error + Send + Sync>>
    where
        M: DecodeWith<Self>,
        Self: Sized,
    {
        self.deserialize(bytes)
    }

    /// Decide whether [`Serializer::deserialize_with_context`] should run after
    /// a successful [`FrameMetadata::parse`] call.
    ///
    /// Serializers that fully decode frames during `parse` can return `false`
    /// to avoid duplicate decoding on the inbound path.
    #[must_use]
    fn should_deserialize_after_parse(&self) -> bool { true }
}

/// Serializer using `bincode` with its standard configuration.
#[derive(Clone, Copy, Debug, Default)]
pub struct BincodeSerializer;

impl MessageCompatibilitySerializer for BincodeSerializer {}

impl Serializer for BincodeSerializer {
    fn serialize<M>(&self, value: &M) -> Result<Vec<u8>, Box<dyn Error + Send + Sync>>
    where
        M: EncodeWith<Self>,
    {
        value.encode_with(self)
    }

    fn deserialize<M>(&self, bytes: &[u8]) -> Result<(M, usize), Box<dyn Error + Send + Sync>>
    where
        M: DecodeWith<Self>,
    {
        M::decode_with(self, bytes, &DeserializeContext::empty())
    }

    fn deserialize_with_context<M>(
        &self,
        bytes: &[u8],
        context: &DeserializeContext<'_>,
    ) -> Result<(M, usize), Box<dyn Error + Send + Sync>>
    where
        M: DecodeWith<Self>,
    {
        M::decode_with(self, bytes, context)
    }

    fn should_deserialize_after_parse(&self) -> bool { false }
}

impl FrameMetadata for BincodeSerializer {
    type Frame = crate::app::Envelope;
    type Error = bincode::error::DecodeError;

    fn parse(&self, src: &[u8]) -> Result<(Self::Frame, usize), Self::Error> {
        crate::app::Envelope::from_bytes(src)
    }
}

#[cfg(feature = "serializer-serde")]
impl SerdeSerializerBridge for BincodeSerializer {
    fn serialize_serde<T: serde::Serialize>(
        &self,
        value: &T,
    ) -> Result<Vec<u8>, Box<dyn Error + Send + Sync>> {
        encode_to_vec(value, config::standard())
            .map_err(|error| Box::new(error) as Box<dyn Error + Send + Sync>)
    }

    fn deserialize_serde<T: serde::de::DeserializeOwned>(
        &self,
        bytes: &[u8],
        _context: &DeserializeContext<'_>,
    ) -> Result<(T, usize), Box<dyn Error + Send + Sync>> {
        decode_from_slice(bytes, config::standard())
            .map_err(|error| Box::new(error) as Box<dyn Error + Send + Sync>)
    }
}
