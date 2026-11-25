//! Generated properties over the copied-byte seams.
//!
//! The `#[case]` tests in the parent module pin the two payload classes the
//! benchmark harness measures. These properties draw arbitrary lengths instead,
//! so a seam that charged a header, rounded a length, or skipped an empty
//! payload fails here rather than being assumed away by two sampled sizes. They
//! live apart from the parent so that file stays within the repository's
//! 400-line limit; the counter lock is unchanged, and every property still runs
//! under `#[serial]` because the counters are process-global.

use bytes::BytesMut;
use proptest::prelude::*;
use serial_test::serial;
use tokio_util::codec::Encoder;
use wireframe::{
    app::{Envelope, benchmark_decode_envelope, benchmark_encode_message_frame},
    benchmark_counters,
    codec::{FrameCodec, LENGTH_HEADER_SIZE, LengthDelimitedFrameCodec},
    serializer::{BincodeSerializer, Serializer},
};
use wireframe_testing::codec_benchmarks::{LARGE_PAYLOAD_BYTES, SMALL_PAYLOAD_BYTES};

/// Payload sizes the generated property draws from.
///
/// The fixed values are the boundaries the `#[case]` tests pin — no payload,
/// one byte, and the two harness classes — and the range lets proptest pick
/// sizes between them that no handwritten case covers.
fn payload_lengths() -> impl Strategy<Value = usize> {
    prop_oneof![
        Just(0_usize),
        Just(1_usize),
        Just(SMALL_PAYLOAD_BYTES),
        Just(LARGE_PAYLOAD_BYTES),
        2_usize..=LARGE_PAYLOAD_BYTES,
    ]
}

proptest! {
    /// Each seam charges exactly the payload, at every payload size.
    ///
    /// The `#[case]` tests pin the two classes the harness measures. This
    /// property covers the sizes between and around them, so a seam that
    /// charged a header, rounded a length, or skipped an empty payload fails
    /// here instead of being assumed away by the two sampled classes.
    #[test]
    #[serial]
    fn seams_charge_one_payload_per_message_at_any_size(len in payload_lengths()) {
        let payload = vec![0x3c_u8; len];
        let envelope = Envelope::new(1, Some(7), payload.clone());

        let framed = BincodeSerializer
            .serialize(&envelope)
            .expect("the default serializer must encode the envelope");
        let encoded_len = framed.len() as u64;

        benchmark_counters::reset();
        let (decoded, consumed) = benchmark_decode_envelope(&BincodeSerializer, &framed)
            .expect("the default serializer must decode its own encoding");
        let inbound = benchmark_counters::snapshot();

        prop_assert_eq!(
            consumed,
            framed.len(),
            "decoding must consume the whole frame"
        );
        prop_assert_eq!(
            decoded.payload_bytes(),
            payload.as_slice(),
            "the decoded envelope must carry the original payload"
        );
        prop_assert_eq!(
            inbound.deserialized,
            len as u64,
            "inbound decode must copy exactly the payload"
        );
        prop_assert_eq!(
            inbound.serialized,
            0,
            "an inbound decode must not serialize"
        );
        prop_assert_eq!(inbound.framed, 0, "an inbound decode must not frame");

        let codec = LengthDelimitedFrameCodec::new(framed.len() + 1024);
        benchmark_counters::reset();
        let frame = benchmark_encode_message_frame(&BincodeSerializer, &codec, &envelope)
            .expect("the default path must encode and frame the envelope");
        let outbound = benchmark_counters::snapshot();

        prop_assert_eq!(
            outbound.serialized,
            encoded_len,
            "the production serializer must copy exactly its own output length"
        );
        prop_assert_eq!(
            outbound.handoff,
            0,
            "the serializer vector must hand off to `Bytes` without copying"
        );
        prop_assert_eq!(
            outbound.framed,
            0,
            "`wrap_payload` must build a frame without writing a transport buffer"
        );
        prop_assert_eq!(
            LengthDelimitedFrameCodec::frame_payload(&frame).len() as u64,
            encoded_len,
            "the frame must carry the serializer's whole output"
        );
    }

    /// The transport encoder charges the framed length once, at any size.
    ///
    /// The `#[case]` matrix frames the two harness payload classes. This
    /// property drives the production encoder over arbitrary sizes, so a
    /// framing pass that rounded a length, charged a header, or skipped an
    /// empty payload fails here instead of being assumed away by two sampled
    /// classes. The transport copy is the one the migration keeps, so `P` is
    /// the pre-migration total for this seam.
    #[test]
    #[serial]
    fn framing_charges_the_framed_length_once_at_any_size(len in payload_lengths()) {
        let envelope = Envelope::new(1, Some(7), vec![0x3c_u8; len]);
        let encoded_len = BincodeSerializer
            .serialize(&envelope)
            .expect("the default serializer must encode the envelope")
            .len();
        let codec = LengthDelimitedFrameCodec::new(encoded_len + 4096);
        let frame = benchmark_encode_message_frame(&BincodeSerializer, &codec, &envelope)
            .expect("the default path must encode and frame the envelope");

        benchmark_counters::reset();
        let mut encoder = codec.encoder();
        let mut wire = BytesMut::new();
        encoder
            .encode(frame, &mut wire)
            .expect("the production encoder must frame the encoded message");
        let framed = benchmark_counters::snapshot();

        prop_assert_eq!(
            framed.framed,
            encoded_len as u64,
            "the transport encoder must charge the serializer output once"
        );
        prop_assert_eq!(
            framed.serialized + framed.handoff,
            0,
            "framing must not reach the serializer or its hand-off"
        );
        prop_assert_eq!(
            wire.len(),
            encoded_len + LENGTH_HEADER_SIZE,
            "the wire form must be the payload plus its length prefix"
        );
    }
}
