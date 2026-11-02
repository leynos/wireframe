//! Asserts that the copied-byte counters observe every in-scope copy seam.
//!
//! Allocation counts cannot see a copy into an already-allocated buffer, so the
//! zero-copy programme's headline claim needs a byte-level assertion. These
//! tests are the regression guard: a change that starts copying a payload, or
//! that silently stops counting one, fails here rather than drifting unnoticed
//! in a benchmark report.
//!
//! The counters are process-global, so every test that measures them runs under
//! `#[serial]`.

#![cfg(feature = "pipeline-benchmarks")]

use rstest::rstest;
use serial_test::serial;
use wireframe::benchmark_counters::CopiedBytes;
use wireframe_testing::codec_benchmarks::{
    BenchmarkWorkload,
    CodecUnderTest,
    PayloadClass,
    SMALL_PAYLOAD_BYTES,
    measure_framed_copies,
    measure_outbound_encode,
};

/// Builds the workload for one codec and payload class.
fn workload(codec: CodecUnderTest, payload_class: PayloadClass) -> BenchmarkWorkload {
    BenchmarkWorkload {
        codec,
        payload_class,
    }
}

#[rstest]
#[case(CodecUnderTest::LengthDelimited, PayloadClass::Small)]
#[case(CodecUnderTest::LengthDelimited, PayloadClass::Large)]
#[case(CodecUnderTest::Hotline, PayloadClass::Small)]
#[case(CodecUnderTest::Hotline, PayloadClass::Large)]
#[serial]
fn outbound_encode_copies_the_payload_once_at_each_seam(
    #[case] codec: CodecUnderTest,
    #[case] payload_class: PayloadClass,
) {
    let workload = workload(codec, payload_class);
    let payload_len = payload_class.len() as u64;

    let measurement = measure_outbound_encode(workload, 1).expect("outbound measurement");

    // The serializer materializes the payload into its own vector.
    assert_eq!(measurement.serialize_only_copies.serialized, payload_len);
    // The harness serializer sizes its output exactly, like bincode's
    // `encode_to_vec`, so the `Bytes::from` hand-off keeps the allocation.
    assert_eq!(measurement.serialize_only_copies.handoff, 0);
    // Serialization alone never reaches a codec, so nothing is framed.
    assert_eq!(measurement.serialize_only_copies.framed, 0);
    // `wrap_payload` builds a frame without writing into a transport buffer, so
    // the wrapping pass copies no bytes beyond the serializer materialization.
    assert_eq!(
        measurement.serialize_and_wrap_copies,
        CopiedBytes {
            serialized: payload_len,
            handoff: 0,
            framed: 0,
        }
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
        }
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

    assert_eq!(total, 2 * payload_len);
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
    assert_eq!(second.serialize_only_copies, first.serialize_only_copies);
    assert_eq!(
        second.serialize_and_wrap_copies,
        first.serialize_and_wrap_copies
    );
    assert_eq!(
        first.serialize_only_copies.serialized,
        SMALL_PAYLOAD_BYTES as u64
    );
}
