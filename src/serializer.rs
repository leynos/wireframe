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
}

/// Serializer using `bincode` with its standard configuration.
#[derive(Clone, Copy, Debug, Default)]
pub struct BincodeSerializer;

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
