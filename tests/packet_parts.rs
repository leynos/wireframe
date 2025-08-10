//! Tests for `PacketParts` conversions and helpers.

use wireframe::app::{Envelope, Packet, PacketParts};

#[test]
fn envelope_from_parts_round_trip() {
    let env = Envelope::new(2, Some(5), vec![1, 2]);
    let parts = env.into_parts();
    let rebuilt = Envelope::from(parts);
    let parts = rebuilt.into_parts();
    assert_eq!(parts.id, 2);
    assert_eq!(parts.correlation_id, Some(5));
    assert_eq!(parts.payload, vec![1, 2]);
}

#[test]
fn inherit_correlation_overwrites_mismatch() {
    let parts = PacketParts::new(1, Some(7), vec![]);
    let inherited = parts.inherit_correlation(Some(8));
    assert_eq!(inherited.correlation_id, Some(8));
}
