//! Shared helpers for applying transport-level fragmentation to packets.

use crate::{
    app::{Packet, PacketParts},
    fragment::{FragmentationError, Fragmenter, encode_fragment_payload},
};

/// Fragment a packet using the provided fragmenter, returning one or more frames.
///
/// Small payloads that fit within the fragment cap are returned unchanged.
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
