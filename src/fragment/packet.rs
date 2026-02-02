//! Shared helpers for applying transport-level fragmentation to packets.
//!
//! This module provides the [`fragment_packet`] function used by both the
//! connection actor and the application layer to split oversized payloads
//! into transport-safe fragment frames.

use super::{FragmentationError, Fragmenter, encode_fragment_payload};
use crate::app::{Packet, PacketParts};

/// Fragment a packet using the provided fragmenter, returning one or more frames.
///
/// Small payloads that fit within the fragment cap are returned unchanged.
///
/// # Errors
///
/// Returns [`FragmentationError`] if fragmenting the payload fails or if
/// encoding the fragment header and payload into an on-wire frame fails.
pub fn fragment_packet<E: Packet>(
    fragmenter: &Fragmenter,
    packet: E,
) -> Result<Vec<E>, FragmentationError> {
    let parts = packet.into_parts();
    let id = parts.id();
    let correlation = parts.correlation_id();
    let payload = parts.payload();

    let batch = fragmenter.fragment_bytes(&payload)?;
    if !batch.is_fragmented() {
        return Ok(vec![E::from_parts(PacketParts::new(
            id,
            correlation,
            payload,
        ))]);
    }

    let mut frames = Vec::with_capacity(batch.len());
    for fragment in batch {
        let (header, payload) = fragment.into_parts();
        let encoded = encode_fragment_payload(header, &payload)?;
        frames.push(E::from_parts(PacketParts::new(id, correlation, encoded)));
    }
    Ok(frames)
}
