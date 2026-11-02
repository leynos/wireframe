//! Allocation baselines for the middleware, request-hook, and outbound stages.
//!
//! The counting allocator lives in `wireframe_testing` so this binary and the
//! codec allocation baseline share one accounting contract. Every label ends
//! with `allocs_<n>`, so an allocation regression is visible in the benchmark
//! name even when timing is stable.

use std::{fmt::Write as _, hint::black_box, time::Duration};

use criterion::{BenchmarkId, Criterion};
use tokio::runtime::Runtime;
use wireframe_testing::{
    benchmark_alloc_support::{CountingAllocator, count_allocations},
    codec_benchmarks::{
        AccessMode,
        BenchmarkWorkload,
        VALIDATION_ITERATIONS,
        benchmark_workloads,
        measure_hook_dispatch,
        measure_middleware_passthrough,
        measure_outbound_serialize,
        measure_outbound_wrap,
        pipeline_label,
        workload_label,
    },
};

/// Process allocator used by this benchmark so the pipeline stages allocate
/// through the same path as the code under test.
#[global_allocator]
static GLOBAL_ALLOCATOR: CountingAllocator = CountingAllocator;

/// Both access modes, in the order the matrix reports them.
const ACCESS_MODES: [AccessMode; 2] = [AccessMode::ReadOnly, AccessMode::Mutating];

/// Builds the current-thread runtime the async stage helpers need.
fn benchmark_runtime() -> Runtime {
    match tokio::runtime::Builder::new_current_thread().build() {
        Ok(runtime) => runtime,
        Err(err) => panic!("tokio runtime construction failed: {err}"),
    }
}

/// Appends an allocation count to a base benchmark label.
///
/// The base label is extended in place so the suffix lands on the label the
/// caller already built, and the helper consumes that label rather than
/// borrowing and copying it.
fn with_allocations(mut base: String, allocations: usize) -> String {
    // Writing into a `String` cannot fail, so the result carries no diagnostic.
    let _ = write!(base, "_allocs_{allocations}");
    base
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
        Err(err) => panic!("outbound serializer baseline failed: {err}"),
    }
}

/// Times one serialize-and-wrap batch, failing the benchmark on error.
fn time_wrap_batch(workload: BenchmarkWorkload, iterations: u64) -> Duration {
    match measure_outbound_wrap(workload, iterations) {
        Ok(measurement) => {
            black_box(measurement.payload_bytes);
            measurement.elapsed
        }
        Err(err) => panic!("outbound encode baseline failed: {err}"),
    }
}

/// Records allocation baselines for the middleware pass-through stage.
fn benchmark_middleware_allocations(c: &mut Criterion, runtime: &Runtime) {
    let mut group = c.benchmark_group("pipeline/middleware_allocations");

    for workload in benchmark_workloads() {
        for mode in ACCESS_MODES {
            let allocations = match count_allocations(|| {
                runtime.block_on(measure_middleware_passthrough(
                    workload,
                    mode,
                    VALIDATION_ITERATIONS,
                ));
                Ok(())
            }) {
                Ok(allocations) => allocations,
                Err(err) => panic!("middleware allocation baseline failed: {err}"),
            };
            let label = with_allocations(
                pipeline_label("middleware_passthrough", workload, mode),
                allocations,
            );

            group.bench_function(BenchmarkId::from_parameter(label), |b| {
                b.iter_custom(|iters| time_middleware_batch(runtime, workload, mode, iters));
            });
        }
    }

    group.finish();
}

/// Records allocation baselines for the request-hook dispatch stage.
fn benchmark_hook_allocations(c: &mut Criterion) {
    let mut group = c.benchmark_group("pipeline/hook_allocations");

    for workload in benchmark_workloads() {
        for mode in ACCESS_MODES {
            let allocations = match count_allocations(|| {
                let _ = measure_hook_dispatch(workload, mode, VALIDATION_ITERATIONS);
                Ok(())
            }) {
                Ok(allocations) => allocations,
                Err(err) => panic!("hook allocation baseline failed: {err}"),
            };
            let label =
                with_allocations(pipeline_label("hook_mutation", workload, mode), allocations);

            group.bench_function(BenchmarkId::from_parameter(label), |b| {
                b.iter_custom(|iters| time_hook_batch(workload, mode, iters));
            });
        }
    }

    group.finish();
}

/// Records allocation baselines for the outbound encode stage.
fn benchmark_outbound_allocations(c: &mut Criterion) {
    let mut group = c.benchmark_group("pipeline/outbound_allocations");

    for workload in benchmark_workloads() {
        let serialize_allocations = match count_allocations(|| {
            let _ = measure_outbound_serialize(workload, VALIDATION_ITERATIONS)?;
            Ok(())
        }) {
            Ok(allocations) => allocations,
            Err(err) => panic!("serializer allocation baseline failed: {err}"),
        };
        let serialize_label = with_allocations(
            workload_label("outbound_serialize", workload),
            serialize_allocations,
        );
        group.bench_function(BenchmarkId::from_parameter(serialize_label), |b| {
            b.iter_custom(|iters| time_serialize_batch(workload, iters));
        });

        let wrap_allocations = match count_allocations(|| {
            let _ = measure_outbound_wrap(workload, VALIDATION_ITERATIONS)?;
            Ok(())
        }) {
            Ok(allocations) => allocations,
            Err(err) => panic!("outbound encode allocation baseline failed: {err}"),
        };
        let wrap_label = with_allocations(
            workload_label("outbound_encode", workload),
            wrap_allocations,
        );
        group.bench_function(BenchmarkId::from_parameter(wrap_label), |b| {
            b.iter_custom(|iters| time_wrap_batch(workload, iters));
        });
    }

    group.finish();
}

/// Entrypoint for the pipeline allocation baselines.
fn main() {
    let runtime = benchmark_runtime();
    let mut criterion = Criterion::default().configure_from_args();
    benchmark_middleware_allocations(&mut criterion, &runtime);
    benchmark_hook_allocations(&mut criterion);
    benchmark_outbound_allocations(&mut criterion);
    criterion.final_summary();
}
