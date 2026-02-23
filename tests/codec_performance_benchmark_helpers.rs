//! Unit tests for codec benchmark workload and measurement helpers.

#![cfg(not(loom))]

use rstest::rstest;

#[path = "common/codec_benchmark_support.rs"]
mod codec_benchmark_support;

use codec_benchmark_support::{
    AllocationBaseline,
    BenchmarkWorkload,
    CodecUnderTest,
    FRAGMENT_PAYLOAD_CAP_BYTES,
    LARGE_PAYLOAD_BYTES,
    PayloadClass,
    SMALL_PAYLOAD_BYTES,
    VALIDATION_ITERATIONS,
    allocation_label,
    benchmark_workloads,
    measure_decode,
    measure_encode,
    measure_fragmentation_overhead,
    payload_for_class,
};

#[rstest]
#[case::small(PayloadClass::Small, SMALL_PAYLOAD_BYTES)]
#[case::large(PayloadClass::Large, LARGE_PAYLOAD_BYTES)]
fn payload_for_class_matches_expected_size(#[case] class: PayloadClass, #[case] expected: usize) {
    let payload = payload_for_class(class);
    assert_eq!(payload.len(), expected);
}

#[test]
fn benchmark_workloads_cover_default_and_custom_codecs() {
    let workloads = benchmark_workloads();
    assert_eq!(
        workloads.len(),
        4,
        "matrix should include 2 codecs x 2 payload classes"
    );

    let has_default_small = workloads.contains(&BenchmarkWorkload {
        codec: CodecUnderTest::LengthDelimited,
        payload_class: PayloadClass::Small,
    });
    let has_default_large = workloads.contains(&BenchmarkWorkload {
        codec: CodecUnderTest::LengthDelimited,
        payload_class: PayloadClass::Large,
    });
    let has_hotline_small = workloads.contains(&BenchmarkWorkload {
        codec: CodecUnderTest::Hotline,
        payload_class: PayloadClass::Small,
    });
    let has_hotline_large = workloads.contains(&BenchmarkWorkload {
        codec: CodecUnderTest::Hotline,
        payload_class: PayloadClass::Large,
    });

    assert!(has_default_small);
    assert!(has_default_large);
    assert!(has_hotline_small);
    assert!(has_hotline_large);
}

#[rstest]
#[case::default_small(BenchmarkWorkload {
    codec: CodecUnderTest::LengthDelimited,
    payload_class: PayloadClass::Small,
})]
#[case::default_large(BenchmarkWorkload {
    codec: CodecUnderTest::LengthDelimited,
    payload_class: PayloadClass::Large,
})]
#[case::hotline_small(BenchmarkWorkload {
    codec: CodecUnderTest::Hotline,
    payload_class: PayloadClass::Small,
})]
#[case::hotline_large(BenchmarkWorkload {
    codec: CodecUnderTest::Hotline,
    payload_class: PayloadClass::Large,
})]
fn encode_measurement_reports_requested_iterations(#[case] workload: BenchmarkWorkload) {
    let measurement = match measure_encode(workload, VALIDATION_ITERATIONS) {
        Ok(value) => value,
        Err(err) => panic!("encode measurement failed: {err}"),
    };
    assert_eq!(measurement.operations, VALIDATION_ITERATIONS);
    assert!(measurement.payload_bytes > 0);
    assert!(measurement.nanos_per_op().is_some());
}

#[rstest]
#[case::default_small(BenchmarkWorkload {
    codec: CodecUnderTest::LengthDelimited,
    payload_class: PayloadClass::Small,
})]
#[case::default_large(BenchmarkWorkload {
    codec: CodecUnderTest::LengthDelimited,
    payload_class: PayloadClass::Large,
})]
#[case::hotline_small(BenchmarkWorkload {
    codec: CodecUnderTest::Hotline,
    payload_class: PayloadClass::Small,
})]
#[case::hotline_large(BenchmarkWorkload {
    codec: CodecUnderTest::Hotline,
    payload_class: PayloadClass::Large,
})]
fn decode_measurement_reports_requested_iterations(#[case] workload: BenchmarkWorkload) {
    let measurement = match measure_decode(workload, VALIDATION_ITERATIONS) {
        Ok(value) => value,
        Err(err) => panic!("decode measurement failed: {err}"),
    };
    assert_eq!(measurement.operations, VALIDATION_ITERATIONS);
    assert!(measurement.payload_bytes > 0);
    assert!(measurement.nanos_per_op().is_some());
}

#[test]
fn fragmentation_overhead_reports_fragmented_and_unfragmented_results() {
    let overhead = match measure_fragmentation_overhead(
        PayloadClass::Large,
        VALIDATION_ITERATIONS,
        FRAGMENT_PAYLOAD_CAP_BYTES,
    ) {
        Ok(value) => value,
        Err(err) => panic!("fragmentation overhead measurement failed: {err}"),
    };

    assert_eq!(overhead.unfragmented.operations, VALIDATION_ITERATIONS);
    assert!(
        overhead.fragmented.operations >= VALIDATION_ITERATIONS,
        "fragmented path should emit at least one wrapped frame per iteration"
    );
    assert!(overhead.fragmented.payload_bytes >= overhead.unfragmented.payload_bytes);
    assert!(overhead.nanos_ratio().is_some());
}

#[test]
fn allocation_label_includes_wrap_and_decode_counters() {
    let workload = BenchmarkWorkload {
        codec: CodecUnderTest::LengthDelimited,
        payload_class: PayloadClass::Small,
    };
    let label = allocation_label(
        workload,
        AllocationBaseline {
            wrap_allocations: 11,
            decode_allocations: 7,
        },
    );

    assert!(label.contains("wrap_allocs_11"));
    assert!(label.contains("decode_allocs_7"));
}
