//! Criterion benchmarks for the middleware, request-hook, and outbound stages.
//!
//! These are the three zero-copy stages that had no harness. Each group reports
//! read-only and mutating paths separately, and the outbound group reports the
//! serializer-only and serializer-plus-`wrap_payload` passes separately. Setup
//! is performed before each timed region by the shared helpers in
//! `wireframe_testing::codec_benchmarks`.

use std::{hint::black_box, time::Duration};

use criterion::{BenchmarkId, Criterion, Throughput};
use tokio::runtime::Runtime;
use wireframe_testing::codec_benchmarks::{
    AccessMode,
    BenchmarkWorkload,
    benchmark_workloads,
    measure_hook_dispatch,
    measure_middleware_passthrough,
    measure_outbound_serialize,
    measure_outbound_wrap,
    pipeline_label,
    workload_label,
};

/// Both access modes, in the order the matrix reports them.
const ACCESS_MODES: [AccessMode; 2] = [AccessMode::ReadOnly, AccessMode::Mutating];

/// Builds the current-thread runtime the async stage helpers need.
///
/// The runtime is created once, outside every timed region, so the measurement
/// never includes executor construction.
fn benchmark_runtime() -> Runtime {
    match tokio::runtime::Builder::new_current_thread().build() {
        Ok(runtime) => runtime,
        Err(err) => panic!("tokio runtime construction failed: {err}"),
    }
}

/// Times one middleware batch for `iter_custom`.
fn time_middleware_batch(
    runtime: &Runtime,
    workload: BenchmarkWorkload,
    mode: AccessMode,
    iterations: u64,
) -> Duration {
    let measurement = runtime.block_on(measure_middleware_passthrough(workload, mode, iterations));
    black_box(measurement.payload_bytes);
    measurement.elapsed
}

/// Times one hook-dispatch batch for `iter_custom`.
fn time_hook_batch(workload: BenchmarkWorkload, mode: AccessMode, iterations: u64) -> Duration {
    let measurement = measure_hook_dispatch(workload, mode, iterations);
    black_box(measurement.payload_bytes);
    measurement.elapsed
}

/// Times one serializer-only batch, failing the benchmark on error.
fn time_serialize_batch(workload: BenchmarkWorkload, iterations: u64) -> Duration {
    match measure_outbound_serialize(workload, iterations) {
        Ok(measurement) => {
            black_box(measurement.payload_bytes);
            measurement.elapsed
        }
        Err(err) => panic!("outbound serializer benchmark failed: {err}"),
    }
}

/// Times one serialize-and-wrap batch, failing the benchmark on error.
fn time_wrap_batch(workload: BenchmarkWorkload, iterations: u64) -> Duration {
    match measure_outbound_wrap(workload, iterations) {
        Ok(measurement) => {
            black_box(measurement.payload_bytes);
            measurement.elapsed
        }
        Err(err) => panic!("outbound encode benchmark failed: {err}"),
    }
}

/// Measures middleware chain traversal for both access modes.
fn benchmark_middleware(c: &mut Criterion, runtime: &Runtime) {
    let mut group = c.benchmark_group("pipeline/middleware");

    // Pass-through borrows the frame and reads its length, so its cost does
    // not scale with the payload. Reporting bytes per second here would divide
    // one message by a few nanoseconds and claim terabytes per second on a
    // 64 KiB payload. Messages per second is the meaningful rate for a stage
    // that forwards a payload without traversing it.
    group.throughput(Throughput::Elements(1));

    for workload in benchmark_workloads() {
        for mode in ACCESS_MODES {
            let label = pipeline_label("middleware_passthrough", workload, mode);
            group.bench_function(BenchmarkId::from_parameter(label), |b| {
                b.iter_custom(|iters| time_middleware_batch(runtime, workload, mode, iters));
            });
        }
    }

    group.finish();
}

/// Measures request-hook dispatch for both access modes.
fn benchmark_hooks(c: &mut Criterion) {
    let mut group = c.benchmark_group("pipeline/hooks");

    // Hook dispatch touches one buffer byte at most, so its cost is constant in
    // the payload size. As with the middleware group, dispatch rate is the
    // meaningful throughput and payload bytes per second would be an artefact
    // of the label.
    group.throughput(Throughput::Elements(1));

    for workload in benchmark_workloads() {
        for mode in ACCESS_MODES {
            let label = pipeline_label("hook_mutation", workload, mode);
            group.bench_function(BenchmarkId::from_parameter(label), |b| {
                b.iter_custom(|iters| time_hook_batch(workload, mode, iters));
            });
        }
    }

    group.finish();
}

/// Measures the outbound serializer-only and serialize-plus-wrap passes.
fn benchmark_outbound(c: &mut Criterion) {
    let mut group = c.benchmark_group("pipeline/outbound");

    for workload in benchmark_workloads() {
        group.throughput(Throughput::Bytes(workload.payload_class.len() as u64));

        let serialize_label = workload_label("outbound_serialize", workload);
        group.bench_function(BenchmarkId::from_parameter(serialize_label), |b| {
            b.iter_custom(|iters| time_serialize_batch(workload, iters));
        });

        let encode_label = workload_label("outbound_encode", workload);
        group.bench_function(BenchmarkId::from_parameter(encode_label), |b| {
            b.iter_custom(|iters| time_wrap_batch(workload, iters));
        });
    }

    group.finish();
}

/// Entrypoint for the pipeline stage benchmarks.
fn main() {
    let runtime = benchmark_runtime();
    let mut criterion = Criterion::default().configure_from_args();
    benchmark_middleware(&mut criterion, &runtime);
    benchmark_hooks(&mut criterion);
    benchmark_outbound(&mut criterion);
    criterion.final_summary();
}
