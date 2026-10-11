//! Criterion benchmarks for the runtime-ownership hot paths of Epic #635.
//!
//! Four scopes, one group each:
//!
//! | Group | Scope | Helper |
//! | --- | --- | --- |
//! | `actor/drain` | connection actor event processing | `measure_actor_drain` |
//! | `push/send` | push handle send path | `measure_push` |
//! | `startup/connection` | application connection startup | `measure_startup` |
//! | `pool/acquisition` | client pool acquisition | `measure_pool_acquisition` |
//!
//! The workloads and the measurement logic live in
//! [`wireframe_testing::runtime_ownership_benchmarks`]; this binary only
//! sequences them for Criterion and pins the sample budget. Keeping the
//! scenarios as data in the support crate is what makes the baseline survive
//! the refactor: the same binary measures the old and the new ownership
//! topology, so a comparison needs no rewrite of the scenario.
//!
//! # Asynchrony
//!
//! Every scenario needs a Tokio runtime. Criterion's `async` feature is not
//! enabled in this workspace, so `Bencher::to_async` is unavailable. Each
//! iteration therefore drives the helper with `Runtime::block_on` inside
//! `iter_custom` and returns the helper's own elapsed duration, which is the
//! arrangement `benches/connection_startup.rs` already uses.
//!
//! A multi-thread runtime is required rather than merely convenient: the
//! contended pool scenarios park callers on a barrier and need real parallelism
//! to exhaust the pool.
//!
//! # Timings are not a gate
//!
//! Criterion stores its own baseline under `target/criterion` and reports a
//! change as a percentage, not a pass or fail. Nothing in continuous
//! integration runs these benchmarks; regression handling is by ratio and
//! manual review, as recorded in `docs/runtime-ownership-baselines.md`.

use std::{hint::black_box, time::Duration};

use criterion::{BenchmarkId, Criterion, Throughput};
use tokio::runtime::Runtime;
use wireframe_testing::runtime_ownership_benchmarks::{
    AcquisitionMode,
    ActorWorkload,
    CancellationMode,
    DEFAULT_MIDDLEWARE_LAYERS,
    DEFAULT_PUSH_QUEUE_CAPACITY,
    DEFAULT_SENDS,
    POOL_SIZES,
    PoolWorkload,
    PushScenario,
    PushWorkload,
    StartupPath,
    StartupWorkload,
    measure_actor_drain,
    measure_pool_acquisition,
    measure_push,
    measure_startup,
};

/// Criterion sample count for every runtime-ownership group.
///
/// Lower than Criterion's default of 100 because each iteration here is
/// expensive: it builds queues, spawns tasks, or opens real loopback
/// connections. A hundred samples per scenario would spend minutes apiece
/// without making the recorded baseline any more useful.
const SAMPLE_SIZE: usize = 20;

/// Criterion measurement time for every runtime-ownership group.
const MEASUREMENT_TIME: Duration = Duration::from_secs(10);

/// Frame counts exercised by the actor drain group.
const ACTOR_FRAME_COUNTS: [usize; 3] = [256, 512, 1024];

/// Cancellation regimes exercised by the actor drain group.
const ACTOR_CANCELLATIONS: [CancellationMode; 3] = [
    CancellationMode::Disabled,
    CancellationMode::CancelledBeforeRun,
    CancellationMode::CancelledDuringDrain,
];

/// Per-queue capacity for every actor drain workload.
///
/// At least half the largest frame count, so no workload blocks on
/// back-pressure and each measures the regime its label claims.
const ACTOR_QUEUE_CAPACITY: usize = 512;

/// Queue capacity for the overflow push workloads.
///
/// Deliberately small: the overflow scenarios fill the queue to capacity first,
/// so a large capacity would spend most of the iteration in the fill rather
/// than in the non-blocking branch under measurement.
const PUSH_OVERFLOW_CAPACITY: usize = 8;

/// Sends per iteration in the overflow push workloads.
const PUSH_OVERFLOW_SENDS: usize = 256;

/// Route counts exercised by the connection startup group.
const STARTUP_ROUTE_COUNTS: [usize; 2] = [1, 2];

/// Connection counts exercised by the connection startup group.
const STARTUP_CONNECTION_COUNTS: [usize; 2] = [1, 4];

/// Pool size used for the contended acquisition workloads.
///
/// Contenders must outnumber slots for the pool to be exhausted, so this stays
/// small enough that [`CONTENDED_POOL_CONTENDERS`] exceeds it comfortably.
const CONTENDED_POOL_SIZE: usize = 4;

/// Contending callers used for the contended acquisition workloads.
const CONTENDED_POOL_CONTENDERS: usize = 16;

/// Convert a workload count to the `u64` Criterion's throughput expects.
///
/// `try_from` rather than `as`: the conversion is lossless only on a 64-bit
/// host, and a silently truncated element count would corrupt the reported
/// throughput rather than fail visibly. Saturating keeps the label usable on an
/// exotic target instead of panicking inside a benchmark.
fn elements(count: usize) -> u64 { u64::try_from(count).unwrap_or(u64::MAX) }

/// Unwrap a benchmark helper's result, terminating with a diagnostic on error.
///
/// A Criterion iteration closure cannot return a `Result`, and a failed
/// workload cannot produce a meaningful sample. Stopping the benchmark is
/// therefore the only honest outcome: recording a time for a scenario that did
/// not run would put a fabricated number in the baseline.
fn expect_bench<T, E: std::fmt::Display>(result: Result<T, E>, context: &str) -> T {
    match result {
        Ok(value) => value,
        Err(error) => panic!("{context}: {error}"),
    }
}

/// The push workloads the send group measures.
///
/// The two uncontended scenarios carry the throughput case and so get the full
/// send count. The three overflow scenarios share a small queue and a shorter
/// run because each exercises one non-blocking branch behind a policy check.
fn push_workloads() -> Vec<PushWorkload> {
    let mut workloads = vec![
        PushWorkload {
            scenario: PushScenario::UncontendedHigh,
            queue_capacity: DEFAULT_PUSH_QUEUE_CAPACITY,
            sends: DEFAULT_SENDS,
        },
        PushWorkload {
            scenario: PushScenario::UncontendedLow,
            queue_capacity: DEFAULT_PUSH_QUEUE_CAPACITY,
            sends: DEFAULT_SENDS,
        },
    ];

    for scenario in [
        PushScenario::OverflowReturnError,
        PushScenario::OverflowDrop,
        PushScenario::OverflowWarnAndDrop,
    ] {
        workloads.push(PushWorkload {
            scenario,
            queue_capacity: PUSH_OVERFLOW_CAPACITY,
            sends: PUSH_OVERFLOW_SENDS,
        });
    }

    workloads
}

/// The pool workloads the acquisition group measures.
///
/// Uncontended acquisition runs the issue's three pool sizes. Contended
/// acquisition runs both fairness policies over one pool size: the issue asks
/// for the policies to be compared, not for every policy to be repeated at
/// every size, and each contended iteration opens a real loopback connection.
fn pool_workloads() -> Vec<PoolWorkload> {
    let mut workloads = POOL_SIZES
        .into_iter()
        .map(|pool_size| PoolWorkload {
            pool_size,
            mode: AcquisitionMode::Uncontended,
            ..PoolWorkload::default()
        })
        .collect::<Vec<_>>();

    for mode in [
        AcquisitionMode::ContendedFifo,
        AcquisitionMode::ContendedRoundRobin,
    ] {
        workloads.push(PoolWorkload {
            pool_size: CONTENDED_POOL_SIZE,
            mode,
            contenders: CONTENDED_POOL_CONTENDERS,
            ..PoolWorkload::default()
        });
    }

    workloads
}

/// Record Criterion timings for the connection actor's drain loop.
///
/// Throughput counts preloaded frames, which is the work each iteration is
/// asked to perform; the cancel-before-run regime therefore reports a rate for
/// frames it declined to move, and its label carries the regime so the two are
/// never confused. The frame count emitted is black-boxed so the compiler
/// cannot elide the drain.
fn benchmark_actor_drain(criterion: &mut Criterion, runtime: &Runtime) {
    let mut group = criterion.benchmark_group("actor/drain");
    group
        .sample_size(SAMPLE_SIZE)
        .measurement_time(MEASUREMENT_TIME);

    for cancellation in ACTOR_CANCELLATIONS {
        for frames in ACTOR_FRAME_COUNTS {
            let workload = ActorWorkload {
                frames,
                queue_capacity: ACTOR_QUEUE_CAPACITY,
                cancellation,
            };
            group.throughput(Throughput::Elements(elements(frames)));
            group.bench_function(BenchmarkId::from_parameter(workload.label()), |bencher| {
                bencher.iter_custom(|iterations| {
                    let measurement = expect_bench(
                        runtime.block_on(measure_actor_drain(workload, iterations)),
                        "actor drain benchmark failed",
                    );
                    black_box(measurement.frames_emitted);
                    measurement.elapsed
                });
            });
        }
    }

    group.finish();
}

/// Record Criterion timings for the push handle's send path.
fn benchmark_push(criterion: &mut Criterion, runtime: &Runtime) {
    let mut group = criterion.benchmark_group("push/send");
    group
        .sample_size(SAMPLE_SIZE)
        .measurement_time(MEASUREMENT_TIME);

    for workload in push_workloads() {
        group.throughput(Throughput::Elements(elements(workload.sends)));
        group.bench_function(BenchmarkId::from_parameter(workload.label()), |bencher| {
            bencher.iter_custom(|iterations| {
                let measurement = expect_bench(
                    runtime.block_on(measure_push(workload, iterations)),
                    "push send benchmark failed",
                );
                // Both counts are black-boxed so the compiler cannot prove the
                // sends unnecessary and elide the work being timed.
                black_box(measurement.accepted);
                black_box(measurement.dead_lettered);
                measurement.elapsed
            });
        });
    }

    group.finish();
}

/// Record Criterion timings for application connection startup.
///
/// The recorded factory and middleware counts already appear in each workload's
/// label, so a timing that improves while the label still claims per-connection
/// preparation is visibly inconsistent rather than a quiet false win.
fn benchmark_connection_startup(criterion: &mut Criterion, runtime: &Runtime) {
    let mut group = criterion.benchmark_group("startup/connection");
    group
        .sample_size(SAMPLE_SIZE)
        .measurement_time(MEASUREMENT_TIME);

    for path in [StartupPath::PerConnectionFactory, StartupPath::PreparedRoot] {
        for routes in STARTUP_ROUTE_COUNTS {
            for connections in STARTUP_CONNECTION_COUNTS {
                let workload = StartupWorkload {
                    path,
                    routes,
                    middleware_layers: DEFAULT_MIDDLEWARE_LAYERS,
                    connections,
                };
                group.throughput(Throughput::Elements(elements(connections)));
                group.bench_function(BenchmarkId::from_parameter(workload.label()), |bencher| {
                    bencher.iter_custom(|iterations| {
                        let measurement = expect_bench(
                            runtime.block_on(measure_startup(workload, iterations)),
                            "connection startup benchmark failed",
                        );
                        black_box(measurement.factory_calls);
                        black_box(measurement.transforms);
                        measurement.elapsed
                    });
                });
            }
        }
    }

    group.finish();
}

/// Record Criterion timings for client pool acquisition.
///
/// Criterion always reports time per iteration, and that is the quantity this
/// group compares: an uncontended iteration is one acquisition burst while a
/// contended iteration is one race across many callers, so the two have no
/// common element count to divide by. Throughput is therefore left unset rather
/// than declared over an inconsistent unit.
fn benchmark_pool_acquisition(criterion: &mut Criterion, runtime: &Runtime) {
    let mut group = criterion.benchmark_group("pool/acquisition");
    group
        .sample_size(SAMPLE_SIZE)
        .measurement_time(MEASUREMENT_TIME);

    for workload in pool_workloads() {
        group.bench_function(BenchmarkId::from_parameter(workload.label()), |bencher| {
            bencher.iter_custom(|iterations| {
                let measurement = expect_bench(
                    runtime.block_on(measure_pool_acquisition(workload, iterations)),
                    "pool acquisition benchmark failed",
                );
                // The scheduler spawn count is the task-churn evidence the issue
                // asks for; black-boxing it keeps the counter read observable.
                black_box(measurement.scheduler_spawns);
                black_box(measurement.alive_task_delta());
                measurement.elapsed
            });
        });
    }

    group.finish();
}

/// Entrypoint for the runtime-ownership baseline benchmarks.
fn main() {
    let runtime = expect_bench(Runtime::new(), "benchmark runtime failed");
    let mut criterion = Criterion::default().configure_from_args();

    benchmark_actor_drain(&mut criterion, &runtime);
    benchmark_push(&mut criterion, &runtime);
    benchmark_connection_startup(&mut criterion, &runtime);
    benchmark_pool_acquisition(&mut criterion, &runtime);

    criterion.final_summary();
}
