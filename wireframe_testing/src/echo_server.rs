//! Echo server utilities for testing client messaging with correlation IDs.
//!
//! Provides [`ServerMode`] and [`process_frame`] for building test servers
//! that either echo envelopes back unchanged or return mismatched correlation
//! IDs for error testing scenarios.

use wireframe::{
    app::{Envelope, Packet},
    correlation::CorrelatableFrame,
    serializer::{BincodeSerializer, Serializer},
};

/// Server mode for testing various correlation ID scenarios.
#[derive(Debug, Clone, Copy, Default)]
pub enum ServerMode {
    /// Echo envelopes back with the same correlation ID.
    #[default]
    Echo,
    /// Return envelopes with a different (mismatched) correlation ID.
    Mismatch,
}

/// Process a single frame and return the response bytes.
///
/// Deserializes an [`Envelope`] from the input bytes, optionally modifies
/// the correlation ID based on the [`ServerMode`], and re-serializes.
///
/// Returns `None` if deserialization or serialization fails.
#[must_use]
pub fn process_frame(mode: ServerMode, bytes: &[u8]) -> Option<Vec<u8>> {
    let (envelope, _): (Envelope, usize) = BincodeSerializer.deserialize(bytes).ok()?;

    let response = match mode {
        ServerMode::Echo => envelope,
        ServerMode::Mismatch => {
            let wrong_id = envelope.correlation_id().map(|id| id.wrapping_add(999));
            let parts = envelope.into_parts();
            Envelope::new(parts.id(), wrong_id, parts.into_payload())
        }
    };

    BincodeSerializer.serialize(&response).ok()
}
