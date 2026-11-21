//! Tests for `PacketParts` conversions and helpers.
#![cfg(not(loom))]
#![forbid(unsafe_code)]

use rstest::rstest;
use wireframe::app::{Envelope, Packet, PacketParts};

/// `Envelope` does not override the `Packet::is_stream_terminator` default, so
/// every envelope shape must report `false`, including an id-0 frame that a
/// protocol using id 0 for its terminator could otherwise mistake.
#[rstest]
#[case::data_envelope(Envelope::new(1, None, vec![42]))]
#[case::id_zero_frame(Envelope::new(0, None, vec![]))]
#[case::correlated_envelope(Envelope::new(1, Some(7), vec![42]))]
fn plain_envelope_is_not_a_stream_terminator(#[case] env: Envelope) {
    assert!(
        !env.is_stream_terminator(),
        "a plain Envelope must never report itself as a stream terminator"
    );
}

#[test]
fn envelope_from_parts_round_trip() {
    let env = Envelope::new(2, Some(5), vec![1, 2]);
    let parts = env.into_parts();
    let rebuilt = Envelope::from(parts);
    let rebuilt_parts = rebuilt.into_parts();
    let id = rebuilt_parts.id();
    let correlation_id = rebuilt_parts.correlation_id();
    let payload = rebuilt_parts.into_payload();
    assert_eq!(id, 2, "id should equal 2");
    assert_eq!(
        correlation_id,
        Some(5),
        "correlation_id should equal Some(5)"
    );
    assert_eq!(payload, vec![1, 2], "payload should equal vec![1, 2]");
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
    assert_eq!(
        got.correlation_id(),
        expected,
        "got.correlation_id() should equal expected"
    );
}
