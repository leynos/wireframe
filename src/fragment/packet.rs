//! Shared helpers for applying transport-level fragmentation to packets.
//!
//! This module provides the [`Fragmentable`] trait and [`fragment_packet`]
//! function used by both the connection actor and the application layer to
//! split oversized payloads into transport-safe fragment frames.

use super::{FragmentationError, Fragmenter, encode_fragment_payload};

/// Component values extracted from or used to build a [`Fragmentable`] packet.
///
/// This type mirrors [`crate::app::PacketParts`] but lives in the fragment
/// module to avoid a layering violation where fragmentation depends on app
/// types.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FragmentParts {
    id: u32,
    correlation_id: Option<u64>,
    payload: Vec<u8>,
}

impl FragmentParts {
    /// Construct a new set of fragment parts.
    #[must_use]
    pub fn new(id: u32, correlation_id: Option<u64>, payload: Vec<u8>) -> Self {
        Self {
            id,
            correlation_id,
            payload,
        }
    }

    /// Return the message identifier.
    #[must_use]
    pub const fn id(&self) -> u32 { self.id }

    /// Retrieve the correlation identifier, if present.
    #[must_use]
    pub const fn correlation_id(&self) -> Option<u64> { self.correlation_id }

    /// Consume the parts and return the raw payload bytes.
    #[must_use]
    pub fn payload(self) -> Vec<u8> { self.payload }
}

/// A packet that can be decomposed into parts and reconstructed for fragmentation.
///
/// This trait captures the minimal interface required by the fragmentation layer
/// to split oversized packets into smaller frames and reassemble them. It
/// intentionally avoids depending on application-layer types like
/// [`crate::app::Packet`].
///
/// Types implementing [`crate::app::Packet`] automatically implement this trait
/// via a blanket implementation in the `app` module.
pub trait Fragmentable: Send + Sync + 'static + Sized {
    /// Consume the packet and return its identifier, correlation id and payload bytes.
    fn into_fragment_parts(self) -> FragmentParts;

    /// Construct a new packet from raw parts.
    fn from_fragment_parts(parts: FragmentParts) -> Self;
}

/// Fragment a packet using the provided fragmenter, returning one or more frames.
///
/// Small payloads that fit within the fragment cap are returned unchanged.
///
/// # Errors
///
/// Returns [`FragmentationError`] if fragmenting the payload fails or if
/// encoding the fragment header and payload into an on-wire frame fails.
pub fn fragment_packet<E: Fragmentable>(
    fragmenter: &Fragmenter,
    packet: E,
) -> Result<Vec<E>, FragmentationError> {
    let parts = packet.into_fragment_parts();
    let id = parts.id();
    let correlation = parts.correlation_id();
    let payload = parts.payload();

    let batch = fragmenter.fragment_bytes(&payload)?;
    if !batch.is_fragmented() {
        return Ok(vec![E::from_fragment_parts(FragmentParts::new(
            id,
            correlation,
            payload,
        ))]);
    }

    let mut frames = Vec::with_capacity(batch.len());
    for fragment in batch {
        let (header, payload) = fragment.into_parts();
        let encoded = encode_fragment_payload(header, &payload)?;
        frames.push(E::from_fragment_parts(FragmentParts::new(
            id,
            correlation,
            encoded,
        )));
    }
    Ok(frames)
}
