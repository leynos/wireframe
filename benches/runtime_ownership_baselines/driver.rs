//! One registration path for every Criterion group, and the per-scope data it
//! consumes.
//!
//! The four scopes differ only in their name, workloads, throughput convention,
//! and measurement call. Those are expressed as a [`BenchScope`] value so there
//! is one loop, not four, which is what makes the groups provably comparable:
//! one scope cannot silently acquire a different sample budget, throughput
//! convention, or measurement call than its siblings.

use std::{hint::black_box, time::Duration};

use criterion::{BenchmarkGroup, BenchmarkId, Criterion, Throughput, measurement::WallTime};
use tokio::runtime::Runtime;
use wireframe_testing::runtime_ownership_benchmarks::{
    ActorWorkload,
    PoolWorkload,
    PushWorkload,
    StartupWorkload,
    measure_actor_drain,
    measure_pool_acquisition,
    measure_push,
    measure_startup,
};

use crate::scenarios::{
    MEASUREMENT_TIME,
    SAMPLE_SIZE,
    actor_workloads,
    elements,
    expect_bench,
    push_workloads,
    startup_workloads,
    uncontended_pool_workloads,
};

/// Run one actor-drain workload for `iterations` and return its elapsed time.
///
/// The emitted-frame count is black-boxed so the compiler cannot prove the drain
/// unnecessary and elide the work being timed.
fn time_actor_drain(runtime: &Runtime, workload: ActorWorkload, iterations: u64) -> Duration {
    let measurement = expect_bench(
        runtime.block_on(measure_actor_drain(workload, iterations)),
        "actor drain benchmark failed",
    );
    black_box(measurement.frames_emitted);
    measurement.elapsed
}

/// Run one push workload for `iterations` and return its elapsed time.
fn time_push(runtime: &Runtime, workload: PushWorkload, iterations: u64) -> Duration {
    let measurement = expect_bench(
        runtime.block_on(measure_push(workload, iterations)),
        "push send benchmark failed",
    );
    // The counts are black-boxed so the compiler cannot prove the sends
    // unnecessary and elide the work being timed. They are not reported from
    // here: Criterion calls this on every iteration, so an event emitted in this
    // function would repeat the counters rather than publish them.
    black_box(measurement.accepted);
    black_box(measurement.dead_lettered);
    black_box(measurement.awaited_capacity_waits);
    measurement.elapsed
}

/// Run one connection-startup workload for `iterations` and return its elapsed
/// time.
fn time_startup(runtime: &Runtime, workload: StartupWorkload, iterations: u64) -> Duration {
    let measurement = expect_bench(
        runtime.block_on(measure_startup(workload, iterations)),
        "connection startup benchmark failed",
    );
    black_box(measurement.factory_calls);
    black_box(measurement.transforms);
    measurement.elapsed
}

/// Run one pool-acquisition workload for `iterations` and return its elapsed
/// time.
fn time_pool_acquisition(runtime: &Runtime, workload: PoolWorkload, iterations: u64) -> Duration {
    let measurement = expect_bench(
        runtime.block_on(measure_pool_acquisition(workload, iterations)),
        "pool acquisition benchmark failed",
    );
    // The scheduler spawn count is the task-churn evidence the issue asks for;
    // black-boxing it keeps the counter read observable. Capacity waits are
    // read alongside it, but they are a different kind of claim: spawns witness
    // the scheduler being entered, waits witness a caller actually having to
    // park for capacity.
    black_box(measurement.scheduler_spawns);
    black_box(measurement.capacity_waits);
    black_box(measurement.alive_task_delta());
    measurement.elapsed
}

/// The name a workload contributes to its Criterion benchmark id.
///
/// Each workload type already has an inherent `label`, but they are four
/// distinct types, so the shared driver below needs one name to reach that
/// method through. The implementations only forward, which keeps the labelling
/// itself owned by the type that knows its own fields.
pub(crate) trait BenchLabel {
    /// This workload's label.
    fn bench_label(self) -> String;
}

impl BenchLabel for ActorWorkload {
    fn bench_label(self) -> String { self.label() }
}

impl BenchLabel for PushWorkload {
    fn bench_label(self) -> String { self.label() }
}

impl BenchLabel for StartupWorkload {
    fn bench_label(self) -> String { self.label() }
}

impl BenchLabel for PoolWorkload {
    fn bench_label(self) -> String { self.label() }
}

/// One Criterion group's scope: what it is called and what it measures.
///
/// The four scopes differ only in these four things, so they are data rather
/// than four parallel functions. Stating a scope as a value is also what makes
/// the groups provably comparable: there is one registration path, so one scope
/// cannot silently acquire a different sample budget, throughput convention, or
/// measurement call than its siblings.
pub(crate) struct BenchScope<W> {
    /// Criterion group name, as it appears in the report.
    name: &'static str,
    /// The workloads this group times, in report order.
    workloads: Vec<W>,
    /// Elements one iteration processes, or `None` to leave throughput unset.
    throughput: fn(W) -> Option<u64>,
    /// The helper that runs one workload for a given iteration count.
    measure: fn(&Runtime, W, u64) -> Duration,
}

/// Open a Criterion group with this baseline's pinned sample budget.
///
/// Every group pins the same budget, so the four groups' figures rest on
/// comparable evidence and a single place decides what that budget is. Extracted
/// rather than repeated because four copies of the same two settings are four
/// places for the budget to drift apart silently.
fn baseline_group<'a>(criterion: &'a mut Criterion, name: &str) -> BenchmarkGroup<'a, WallTime> {
    let mut group = criterion.benchmark_group(name);
    group
        .sample_size(SAMPLE_SIZE)
        .measurement_time(MEASUREMENT_TIME);
    group
}

/// Time every workload in `scope`, then close its group.
///
/// This is the single registration path all four scopes share. Each iteration
/// drives `scope.measure` with the benchmark's own `Runtime` inside
/// `iter_custom`, because Criterion's `async` feature is not enabled here; see
/// the module docs. The returned duration is the helper's own elapsed
/// measurement, not Criterion's wall clock around the whole batch.
pub(crate) fn run_scope<W: BenchLabel + Copy>(
    criterion: &mut Criterion,
    runtime: &Runtime,
    scope: BenchScope<W>,
) {
    let mut group = baseline_group(criterion, scope.name);

    for workload in scope.workloads {
        if let Some(elements) = (scope.throughput)(workload) {
            group.throughput(Throughput::Elements(elements));
        }
        group.bench_function(
            BenchmarkId::from_parameter(workload.bench_label()),
            |bencher| {
                bencher.iter_custom(|iterations| (scope.measure)(runtime, workload, iterations));
            },
        );
    }

    group.finish();
}

/// The actor-drain scope: the connection actor's drain loop.
///
/// A cancellation regime is crossed with every frame count. Throughput counts
/// preloaded frames, which is the work each iteration is asked to perform; the
/// cancel-before-run regime therefore reports a rate for frames it declined to
/// move, and its label carries the regime so the two are never confused.
pub(crate) fn actor_scope() -> BenchScope<ActorWorkload> {
    BenchScope {
        name: "actor/drain",
        workloads: actor_workloads(),
        throughput: |workload| Some(elements(workload.frames)),
        measure: time_actor_drain,
    }
}

/// The push scope: the handle's send path.
///
/// Throughput counts sends, which is the work each iteration is asked to
/// perform. Kept as a value alongside the other scopes so the four groups are
/// registered by one path and cannot drift apart in budget or convention.
pub(crate) fn push_scope() -> BenchScope<PushWorkload> {
    BenchScope {
        name: "push/send",
        workloads: push_workloads(),
        throughput: |workload| Some(elements(workload.sends)),
        measure: time_push,
    }
}

/// The connection-startup scope.
///
/// The recorded factory and middleware counts already appear in each workload's
/// label, so a timing that improves while the label still claims per-connection
/// preparation is visibly inconsistent rather than a quiet false win. The route
/// and connection grids are flattened in [`startup_workloads`] rather than
/// nested as two more loops, which keeps the registration closure shallow enough
/// for the workspace's nesting limit.
pub(crate) fn startup_scope() -> BenchScope<StartupWorkload> {
    BenchScope {
        name: "startup/connection",
        workloads: startup_workloads(),
        throughput: |workload| Some(elements(workload.connections)),
        measure: time_startup,
    }
}

/// The uncontended pool-acquisition scope.
///
/// Criterion always reports time per iteration, and that is the quantity this
/// group compares: an iteration is one acquire/release burst, so the element
/// count varies only with the workload's `acquisitions` parameter. Throughput is
/// left unset rather than declared over a unit that changes between scenarios,
/// which is why this scope's `throughput` returns `None`.
pub(crate) fn pool_scope() -> BenchScope<PoolWorkload> {
    BenchScope {
        name: "pool/acquisition",
        workloads: uncontended_pool_workloads(),
        throughput: |_workload| None,
        measure: time_pool_acquisition,
    }
}
