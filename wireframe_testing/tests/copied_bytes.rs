//! Asserts that the copied-byte counters observe every in-scope copy seam.
//!
//! Allocation counts cannot see a copy into an already-allocated buffer, so the
//! zero-copy programme's headline claim needs a byte-level assertion. These
//! tests are the regression guard: a change that starts copying a payload, or
//! that silently stops counting one, fails here rather than drifting unnoticed
//! in a benchmark report.
//!
//! Some assertions here state the pre-migration totals on purpose: roadmap
//! items `11.1.1` and `11.1.2` remove the copies they count, so their failure
//! is the signal that the migration arrived. The register in
//! `docs/zero-copy-benchmark-baselines.md` records which test each item flips;
//! the [`inversion_register`] module checks that it names tests in this file
//! that still exist.
//!
//! The counters are process-global, so every test that measures them runs under
//! `#[serial]`.

#![cfg(feature = "pipeline-benchmarks")]
#![forbid(unsafe_code)]

use proptest::prelude::*;
use rstest::rstest;
use serial_test::serial;
use wireframe::{
    app::{Envelope, benchmark_decode_envelope, benchmark_encode_message_frame},
    benchmark_counters::{self, CopiedBytes},
    codec::{FrameCodec, LengthDelimitedFrameCodec},
    serializer::{BincodeSerializer, Serializer},
};
use wireframe_testing::codec_benchmarks::{
    BenchmarkWorkload,
    CodecUnderTest,
    LARGE_PAYLOAD_BYTES,
    PayloadClass,
    SMALL_PAYLOAD_BYTES,
    measure_framed_copies,
    measure_outbound_encode,
    payload_for_class,
};

// A test-file crate root resolves `mod` beside itself rather than in its own
// directory, so the split module needs an explicit path.
#[path = "copied_bytes/inversion_register.rs"]
mod inversion_register;

/// Builds the workload for one codec and payload class.
const fn workload(codec: CodecUnderTest, payload_class: PayloadClass) -> BenchmarkWorkload {
    BenchmarkWorkload {
        codec,
        payload_class,
    }
}

/// Builds the default-path envelope for one payload class.
fn default_envelope(payload_class: PayloadClass) -> Envelope {
    Envelope::new(1, Some(7), payload_for_class(payload_class).to_vec())
}

/// The harness's own `VectorSerializer` is measured here, not the production
/// `BincodeSerializer`. `default_serializer_and_handoff_copy_the_payload_once`
/// covers the serializer the default path actually uses.
#[rstest]
#[case(CodecUnderTest::LengthDelimited, PayloadClass::Small)]
#[case(CodecUnderTest::LengthDelimited, PayloadClass::Large)]
#[case(CodecUnderTest::Hotline, PayloadClass::Small)]
#[case(CodecUnderTest::Hotline, PayloadClass::Large)]
#[serial]
fn harness_serializer_copies_the_payload_once_at_each_seam(
    #[case] codec: CodecUnderTest,
    #[case] payload_class: PayloadClass,
) {
    let workload = workload(codec, payload_class);
    let payload_len = payload_class.len() as u64;

    let measurement = measure_outbound_encode(workload, 1).expect("outbound measurement");

    // The serializer materializes the payload into its own vector.
    assert_eq!(
        measurement.serialize_only_copies.serialized, payload_len,
        "measurement.serialize_only_copies.serialized should equal payload_len"
    );
    // The harness serializer sizes its output exactly, like bincode's
    // `encode_to_vec`, so the `Bytes::from` hand-off keeps the allocation.
    assert_eq!(
        measurement.serialize_only_copies.handoff, 0,
        "measurement.serialize_only_copies.handoff should equal 0"
    );
    // Serialization alone never reaches a codec, so nothing is framed.
    assert_eq!(
        measurement.serialize_only_copies.framed, 0,
        "measurement.serialize_only_copies.framed should equal 0"
    );
    // `wrap_payload` builds a frame without writing into a transport buffer, so
    // the wrapping pass copies no bytes beyond the serializer materialization.
    assert_eq!(
        measurement.serialize_and_wrap_copies,
        CopiedBytes {
            serialized: payload_len,
            handoff: 0,
            framed: 0,
            deserialized: 0,
        },
        "measurement.serialize_and_wrap_copies should equal CopiedBytes {{ serialized: \
         payload_len, handoff: 0, framed: 0, deserialized: 0, }}"
    );

    // The transport copy belongs to `Encoder::encode`, which the framing probe
    // runs on its own.
    let framed = measure_framed_copies(workload, 1).expect("framed copy measurement");
    assert_eq!(
        framed,
        CopiedBytes {
            serialized: 0,
            handoff: 0,
            framed: payload_len,
            deserialized: 0,
        },
        "framed should equal CopiedBytes {{ serialized: 0, handoff: 0, framed: payload_len, \
         deserialized: 0, }}"
    );
}

#[rstest]
#[case(CodecUnderTest::LengthDelimited, PayloadClass::Small)]
#[case(CodecUnderTest::LengthDelimited, PayloadClass::Large)]
#[case(CodecUnderTest::Hotline, PayloadClass::Small)]
#[case(CodecUnderTest::Hotline, PayloadClass::Large)]
#[serial]
fn outbound_path_copies_the_payload_twice_per_message(
    #[case] codec: CodecUnderTest,
    #[case] payload_class: PayloadClass,
) {
    // The pre-migration default path copies the payload twice: once when the
    // serializer materializes its `Vec<u8>`, once when the codec writes the
    // frame into the transport buffer. The migration removes the first copy and
    // keeps the second, so this total is the number a post-migration capture
    // must halve.
    let workload = workload(codec, payload_class);
    let payload_len = payload_class.len() as u64;

    let outbound = measure_outbound_encode(workload, 1).expect("outbound measurement");
    let framed = measure_framed_copies(workload, 1).expect("framed copy measurement");
    let total = outbound.serialize_and_wrap_copies.total() + framed.total();

    assert_eq!(total, 2 * payload_len, "total should equal 2 * payload_len");
}

/// One inbound message copies its payload exactly once, into the envelope.
///
/// The counter is the only place the inbound copy is visible: the framed
/// reader borrows its payload, so an allocation count cannot tell a decoder
/// that shares the frame's bytes from one that materializes its own.
#[rstest]
#[case(PayloadClass::Small)]
#[case(PayloadClass::Large)]
#[serial]
fn inbound_decode_copies_the_payload_once(#[case] payload_class: PayloadClass) {
    let payload_len = payload_class.len() as u64;
    let frame_payload = BincodeSerializer
        .serialize(&default_envelope(payload_class))
        .expect("the default serializer must encode its own envelope");

    benchmark_counters::reset();
    let (decoded, consumed) = benchmark_decode_envelope(&BincodeSerializer, &frame_payload)
        .expect("the default serializer must decode its own encoding");
    let copied = benchmark_counters::snapshot();

    assert_eq!(
        consumed,
        frame_payload.len(),
        "decoding must consume the whole frame payload"
    );
    assert_eq!(
        decoded.payload_bytes().len() as u64,
        payload_len,
        "the decoded envelope must carry the original payload"
    );
    assert_eq!(
        copied.deserialized, payload_len,
        "inbound decode must copy exactly one payload; roadmap item 11.1.1 drives this to zero"
    );
    assert_eq!(
        copied.serialized, 0,
        "an inbound decode must not reach an outbound serialization seam"
    );
    assert_eq!(
        copied.framed, 0,
        "an inbound decode must not reach the framing seam"
    );
}

/// The production serializer and its hand-off copy the payload exactly once.
///
/// This is the assertion the harness-serializer case could not make: it uses
/// the `BincodeSerializer` the default path is built with, and the codec that
/// precedes framing.
#[rstest]
#[case(PayloadClass::Small)]
#[case(PayloadClass::Large)]
#[serial]
fn default_serializer_and_handoff_copy_the_payload_once(#[case] payload_class: PayloadClass) {
    let payload_len = payload_class.len() as u64;
    let envelope = default_envelope(payload_class);
    // The encoded length includes the envelope's identifiers, so the
    // expectation comes from the same serializer rather than from `P`.
    let encoded_len = BincodeSerializer
        .serialize(&envelope)
        .expect("the default serializer must encode the envelope")
        .len() as u64;
    let codec = LengthDelimitedFrameCodec::new(LARGE_PAYLOAD_BYTES + 4096);

    benchmark_counters::reset();
    let frame = benchmark_encode_message_frame(&BincodeSerializer, &codec, &envelope)
        .expect("the default path must encode and frame the envelope");
    let copied = benchmark_counters::snapshot();

    assert_eq!(
        copied.serialized, encoded_len,
        "the production serializer must copy exactly its own output length"
    );
    assert!(
        copied.serialized >= payload_len,
        "the serializer output must be at least the payload: {} bytes for a {payload_len}-byte \
         payload",
        copied.serialized
    );
    assert_eq!(
        copied.handoff, 0,
        "the serializer vector must hand off to `Bytes` without copying"
    );
    assert_eq!(
        copied.framed, 0,
        "`wrap_payload` must build a frame without writing a transport buffer"
    );
    assert_eq!(
        LengthDelimitedFrameCodec::frame_payload(&frame).len() as u64,
        encoded_len,
        "the frame must carry the serializer's whole output"
    );
}

/// The default serializer leaves no spare capacity for `Bytes::from` to pay
/// for.
#[rstest]
#[case(PayloadClass::Small)]
#[case(PayloadClass::Large)]
#[serial]
fn default_serializer_leaves_no_spare_capacity(#[case] payload_class: PayloadClass) {
    let serialized = BincodeSerializer
        .serialize(&default_envelope(payload_class))
        .expect("the default serializer must encode the envelope");
    let expected = serialized.len();
    assert_eq!(
        serialized.capacity(),
        expected,
        "bincode sizes its output before allocating, so length and capacity must match"
    );

    benchmark_counters::reset();
    let bytes = benchmark_counters::bytes_from_vec(serialized);

    assert_eq!(
        bytes.len(),
        expected,
        "the hand-off must preserve the serializer output"
    );
    assert_eq!(
        benchmark_counters::snapshot().handoff,
        0,
        "an exact-capacity vector must hand off without copying"
    );
}

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
}

#[test]
#[serial]
fn successive_measurements_do_not_accumulate_counter_values() {
    let workload = workload(CodecUnderTest::LengthDelimited, PayloadClass::Small);

    let first = measure_outbound_encode(workload, 2).expect("first measurement");
    let second = measure_outbound_encode(workload, 2).expect("second measurement");

    // The counters are reset per measurement, so the second run reports the same
    // per-operation copy cost rather than twice it. Timings are deliberately
    // excluded: they vary between runs and are not the property under test.
    assert_eq!(
        second.serialize_only_copies, first.serialize_only_copies,
        "second.serialize_only_copies should equal first.serialize_only_copies"
    );
    assert_eq!(
        second.serialize_and_wrap_copies, first.serialize_and_wrap_copies,
        "second.serialize_and_wrap_copies should equal first.serialize_and_wrap_copies"
    );
    assert_eq!(
        first.serialize_only_copies.serialized, SMALL_PAYLOAD_BYTES as u64,
        "first.serialize_only_copies.serialized should equal SMALL_PAYLOAD_BYTES as u64"
    );
}
