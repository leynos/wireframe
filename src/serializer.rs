//! Message serialization traits.
//!
//! This module defines the [`Serializer`] trait enabling applications to plug in
//! custom encoding formats. A basic [`BincodeSerializer`] implementation is
//! provided as the default.

use std::error::Error;

use crate::message::Message;

/// Trait for serializing and deserializing messages.
pub trait Serializer {
    /// Serialize `value` into a byte vector.
    ///
    /// # Errors
    ///
    /// Returns an error if the value cannot be serialized.
    fn serialize<M: Message>(&self, value: &M) -> Result<Vec<u8>, Box<dyn Error + Send + Sync>>;

    /// Deserialize a message from `bytes`, returning the message and bytes consumed.
    ///
    /// # Errors
    ///
    /// Returns an error if the bytes cannot be parsed into a message.
    fn deserialize<M: Message>(
        &self,
        bytes: &[u8],
    ) -> Result<(M, usize), Box<dyn Error + Send + Sync>>;
}

/// Serializer using `bincode` with its standard configuration.
#[derive(Clone, Copy, Debug, Default)]
pub struct BincodeSerializer;

impl Serializer for BincodeSerializer {
    fn serialize<M: Message>(&self, value: &M) -> Result<Vec<u8>, Box<dyn Error + Send + Sync>> {
        value
            .to_bytes()
            .map_err(|e| Box::new(e) as Box<dyn Error + Send + Sync>)
    }

    fn deserialize<M: Message>(
        &self,
        bytes: &[u8],
    ) -> Result<(M, usize), Box<dyn Error + Send + Sync>> {
        M::from_bytes(bytes).map_err(|e| Box::new(e) as Box<dyn Error + Send + Sync>)
    }
}
