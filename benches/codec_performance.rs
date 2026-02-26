//! Criterion benchmarks for codec throughput, latency, and fragmentation overhead.
//!
//! This benchmark suite covers:
//! - encode and decode throughput/latency for small and large payloads,
//! - default and custom codec paths (`LengthDelimitedFrameCodec`, `HotlineFrameCodec`), and
//! - fragmented versus unfragmented payload-wrapping overhead.

use criterion::{BenchmarkId, Criterion, Throughput, black_box};

#[path = "../tests/common/codec_benchmark_support.rs"]
mod codec_benchmark_support;

#[path = "../tests/common/codec_fragmentation_benchmark_support.rs"]
mod codec_fragmentation_benchmark_support;

use codec_benchmark_support::{
    Measurement,
    PayloadClass,
    VALIDATION_ITERATIONS,
    benchmark_workloads,
    measure_decode,
    measure_encode,
};
use codec_fragmentation_benchmark_support::{
    FRAGMENT_PAYLOAD_CAP_BYTES,
    MeasurementExt as _,
    measure_fragmentation_overhead,
    measure_fragmented_wrap,
    measure_unfragmented_wrap,
};

fn fragmented_measurement(payload_class: PayloadClass, iterations: u64) -> Measurement {
    match measure_fragmented_wrap(payload_class, iterations, FRAGMENT_PAYLOAD_CAP_BYTES) {
        Ok(measurement) => measurement,
        Err(err) => panic!("fragmented benchmark setup failed: {err}"),
    }
}

fn benchmark_encode(c: &mut Criterion) {
    let mut group = c.benchmark_group("codec/encode");

    for workload in benchmark_workloads() {
        let bytes = workload.payload_class.len() as u64;
        group.throughput(Throughput::Bytes(bytes));
        group.bench_function(BenchmarkId::from_parameter(workload.label()), |b| {
            b.iter_custom(|iters| match measure_encode(workload, iters) {
                Ok(measurement) => {
                    black_box(measurement.payload_bytes);
                    measurement.elapsed
                }
                Err(err) => panic!("encode benchmark setup failed: {err}"),
            });
        });
    }

    group.finish();
}

fn benchmark_decode(c: &mut Criterion) {
    let mut group = c.benchmark_group("codec/decode");

    for workload in benchmark_workloads() {
        let bytes = workload.payload_class.len() as u64;
        group.throughput(Throughput::Bytes(bytes));
        group.bench_function(BenchmarkId::from_parameter(workload.label()), |b| {
            b.iter_custom(|iters| match measure_decode(workload, iters) {
                Ok(measurement) => {
                    black_box(measurement.payload_bytes);
                    measurement.elapsed
                }
                Err(err) => panic!("decode benchmark setup failed: {err}"),
            });
        });
    }

    group.finish();
}

fn benchmark_fragmentation_overhead(c: &mut Criterion) {
    let mut group = c.benchmark_group("codec/fragmentation_overhead");

    for payload_class in [PayloadClass::Small, PayloadClass::Large] {
        let baseline = match measure_fragmentation_overhead(
            payload_class,
            VALIDATION_ITERATIONS,
            FRAGMENT_PAYLOAD_CAP_BYTES,
        ) {
            Ok(result) => result,
            Err(err) => panic!("fragmentation baseline setup failed: {err}"),
        };
        let ratio = baseline.nanos_ratio().unwrap_or_else(|| {
            panic!(
                "fragmentation baseline has no valid nanos_ratio for payload_class={}",
                payload_class.label()
            )
        });
        black_box(baseline.unfragmented.nanos_per_op());
        black_box(baseline.fragmented.nanos_per_op());

        group.bench_function(
            BenchmarkId::new("unfragmented", payload_class.label()),
            |b| {
                b.iter_custom(|iters| {
                    let measurement = measure_unfragmented_wrap(payload_class, iters);
                    black_box(measurement.payload_bytes);
                    measurement.elapsed
                });
            },
        );

        group.bench_function(
            BenchmarkId::new(
                "fragmented",
                format!("{}_ratio_{ratio:.3}", payload_class.label()),
            ),
            |b| {
                b.iter_custom(|iters| {
                    let measurement = fragmented_measurement(payload_class, iters);
                    black_box(measurement.payload_bytes);
                    measurement.elapsed
                });
            },
        );
    }

    group.finish();
}

/// Entrypoint for codec throughput, latency, and fragmentation benchmarks.
fn main() {
    let mut criterion = Criterion::default().configure_from_args();
    benchmark_encode(&mut criterion);
    benchmark_decode(&mut criterion);
    benchmark_fragmentation_overhead(&mut criterion);
    criterion.final_summary();
}
