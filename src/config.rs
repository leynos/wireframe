//! Application configuration types.
//!
//! This module defines enums and helpers for selecting
//! serialization formats used by `wireframe` when encoding
//! and decoding messages.
use bincode::error::{DecodeError, EncodeError};

use crate::message::Message;

/// Serialization formats supported by `wireframe`.
#[derive(Clone, Copy)]
pub enum SerializationFormat {
    /// Use `bincode` with its standard configuration.
    Bincode,
}

impl SerializationFormat {
    /// Serialize a message into a byte vector.
    ///
    /// # Errors
    ///
    /// Returns an [`EncodeError`] if serialization fails.
    pub fn serialize<M: Message>(self, value: &M) -> Result<Vec<u8>, EncodeError> {
        match self {
            SerializationFormat::Bincode => value.to_bytes(),
        }
    }

    /// Deserialize a message from a byte slice.
    ///
    /// # Errors
    ///
    /// Returns a [`DecodeError`] if deserialization fails.
    pub fn deserialize<M: Message>(self, bytes: &[u8]) -> Result<(M, usize), DecodeError> {
        match self {
            SerializationFormat::Bincode => M::from_bytes(bytes),
        }
    }
}
