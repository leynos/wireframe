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

#[rstest::rstest(
    start, source, expected,
    case(PacketParts::new(1, None, vec![]), Some(42), Some(42)),
    case(PacketParts::new(1, Some(7), vec![]), None, Some(7)),
    case(PacketParts::new(1, None, vec![]), None, None),
    case(PacketParts::new(1, Some(7), vec![]), Some(8), Some(8)),
)]
fn inherit_variants(start: PacketParts, source: Option<u64>, expected: Option<u64>) {
    let got = start.inherit_correlation(source);
    assert_eq!(got.correlation_id, expected);
}
