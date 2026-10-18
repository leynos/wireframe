//! Criterion benchmarks for the runtime-ownership hot paths of Epic #635.
//!
//! Four scopes, one group each:
//!
//! | Group | Scope | Helper |
//! | --- | --- | --- |
//! | `actor/drain` | connection actor event processing | `measure_actor_drain` |
//! | `push/send` | push handle send path | `measure_push` |
//! | `startup/connection` | application connection startup | `measure_startup` |
//! | `pool/acquisition` | uncontended client pool acquisition | `measure_pool_acquisition` |
//!
//! Contended pool acquisition is deliberately absent from that table. It is
//! reported after `Criterion::final_summary` as bounded `tracing` events rather
//! than timed, because sustained contention intermittently stalls the pool
//! acquire path: Criterion would block during warm-up sampling a scenario that
//! never returns, taking the whole run with it. It is sampled repeatedly rather
//! than once — a single elapsed value for a scenario that also fails is
//! ambiguous — and the run is reported as a latency distribution beside the
//! pool's churn counters, with failed bursts counted rather than retried into
//! the samples. Those counters are what the issue asks for at this scope, and
//! they are recorded as observations, not as timings. See
//! `docs/runtime-ownership-baselines.md`.
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

use criterion::{BenchmarkGroup, BenchmarkId, Criterion, Throughput, measurement::WallTime};
use tokio::runtime::Runtime;
use wireframe_testing::runtime_ownership_benchmarks::{
    AcquisitionMode,
    ActorWorkload,
    CONTENDED_BURST_TIMEOUT,
    CONTENDED_LATENCY_SAMPLES,
    CancellationMode,
    ContendedLatency,
    DEFAULT_MIDDLEWARE_LAYERS,
    DEFAULT_PUSH_QUEUE_CAPACITY,
    POOL_SIZES,
    PoolWorkload,
    PushScenario,
    PushWorkload,
    StartupPath,
    StartupWorkload,
    measure_actor_drain,
    measure_contended_latency,
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
///
/// The application shape is held small on purpose: the axis this scope has to
/// resolve is how the two preparation paths differ as connections multiply, and
/// a route count large enough to dominate the figure would obscure that.
/// [`STARTUP_CONNECTION_COUNTS`] carries the comparison.
const STARTUP_ROUTE_COUNTS: [usize; 2] = [1, 2];

/// Connection counts exercised by the connection startup group.
///
/// Crossed with [`STARTUP_ROUTE_COUNTS`], so the group covers 1, 4, and 16
/// connections. The largest case exists because the two paths only diverge with
/// connection count: the prepared root pays preparation once and amortizes it,
/// so a one- and a four-connection run show a difference but not its shape. Sixteen
/// is the same ceiling the actor and pool scopes use, which keeps the four
/// scopes' "large" cases at one scale rather than four unrelated ones. It is not
/// a claim about a particular deployment; it is the point on this axis where the
/// per-connection path's linear growth is unambiguous against the prepared
/// root's sub-linear growth.
const STARTUP_CONNECTION_COUNTS: [usize; 3] = [1, 4, 16];

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

/// Send count for the push no-back-pressure control.
///
/// Must not exceed the queue capacity, because that is what makes the control
/// a control: a producer sending no more frames than the queue holds cannot
/// park whatever the consumer is doing. See [`PUSH_QUEUE_CAPACITY`].
const PUSH_CONTROL_SENDS: usize = 64;

/// Send count for the push throughput workloads.
///
/// Deliberately larger than the queue capacity, so the producer can outrun a
/// consumer. Whether it does is the observation, reported through
/// [`PushMeasurement::awaited_capacity_waits`] rather than asserted.
const PUSH_DRAINED_SENDS: usize = 1024;

/// Queue capacity for the push backpressure workload.
///
/// Small, and much smaller than [`PUSH_DRAINED_SENDS`], because the scenario
/// has no consumer: the queue must fill within the run for the producer to park
/// at all. A capacity that could hold the whole run would measure the
/// empty-queue path under a backpressure label.
const PUSH_BACKPRESSURE_CAPACITY: usize = 8;

/// The push workloads the send group measures.
///
/// The awaited scenarios carry the throughput case and so get the full send
/// count. The three overflow scenarios share a small queue and a shorter run
/// because each exercises one non-blocking branch behind a policy check.
fn push_workloads() -> Vec<PushWorkload> {
    let mut workloads = vec![
        // The no-back-pressure control: the queue holds the whole run, so a
        // zero wait count is a property of the workload rather than luck.
        PushWorkload {
            scenario: PushScenario::UncontendedHigh,
            queue_capacity: PUSH_CONTROL_SENDS,
            sends: PUSH_CONTROL_SENDS,
        },
        // The backpressure case: nothing drains the queue, so the producer must
        // park against a full one. Kept separate from the drained scenarios
        // because a draining consumer does not guarantee the producer never
        // waits, and `reported_queue_full` does not observe awaited waits at all.
        PushWorkload {
            scenario: PushScenario::BackpressureHigh,
            queue_capacity: PUSH_BACKPRESSURE_CAPACITY,
            sends: PUSH_CONTROL_SENDS,
        },
        // The throughput case: the queue cannot hold the run, so the producer
        // may park and the reported wait count says whether it did.
        PushWorkload {
            scenario: PushScenario::DrainedHigh,
            queue_capacity: DEFAULT_PUSH_QUEUE_CAPACITY,
            sends: PUSH_DRAINED_SENDS,
        },
        PushWorkload {
            scenario: PushScenario::DrainedLow,
            queue_capacity: DEFAULT_PUSH_QUEUE_CAPACITY,
            sends: PUSH_DRAINED_SENDS,
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

/// The connection-startup workloads the startup group times.
///
/// Both preparation paths are crossed with every route and connection count, in
/// the order the recorded baseline table lists them. The path is the outer loop
/// so the two paths' rows stay grouped in the Criterion report rather than
/// interleaved, which is what makes the per-connection-versus-prepared contrast
/// readable at a glance.
fn startup_workloads() -> Vec<StartupWorkload> {
    [StartupPath::PerConnectionFactory, StartupPath::PreparedRoot]
        .into_iter()
        .flat_map(|path| {
            STARTUP_ROUTE_COUNTS.into_iter().flat_map(move |routes| {
                STARTUP_CONNECTION_COUNTS
                    .into_iter()
                    .map(move |connections| StartupWorkload {
                        path,
                        routes,
                        middleware_layers: DEFAULT_MIDDLEWARE_LAYERS,
                        connections,
                    })
            })
        })
        .collect()
}

/// The uncontended pool workloads the acquisition group times.
///
/// Only uncontended acquisition is timed. Contended acquisition is excluded
/// because it is not reliable enough to sample: sustained contention
/// intermittently stalls the acquire path outright, so a Criterion run would
/// block during warm-up rather than record a number. That defect is pre-existing
/// and belongs to ADR 013 / issue #647; [`observe_contended_acquisition`]
/// records its counters under a timeout instead of timing it. See
/// `docs/runtime-ownership-baselines.md`.
fn uncontended_pool_workloads() -> Vec<PoolWorkload> {
    POOL_SIZES
        .into_iter()
        .map(|pool_size| PoolWorkload {
            pool_size,
            mode: AcquisitionMode::Uncontended,
            ..PoolWorkload::default()
        })
        .collect()
}

/// The contended pool workloads reported as bounded observations, not timings.
///
/// Both fairness policies run over one pool size: the issue asks for the
/// policies to be compared, not for every policy to be repeated at every size.
fn contended_pool_workloads() -> Vec<PoolWorkload> {
    [
        AcquisitionMode::ContendedFifo,
        AcquisitionMode::ContendedRoundRobin,
    ]
    .into_iter()
    .map(|mode| PoolWorkload {
        pool_size: CONTENDED_POOL_SIZE,
        mode,
        contenders: CONTENDED_POOL_CONTENDERS,
        ..PoolWorkload::default()
    })
    .collect()
}

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

/// Emit each push workload's awaited-wait counters as an observation.
///
/// A timing alone cannot say which path a drained scenario took: a fast run may
/// have found the queue's fast path or its back-pressure path, and the two are
/// not comparable. The wait counters say which one it was, and they are not
/// timings, so `tracing` carries them rather than the Criterion report.
///
/// Taken from a one-iteration measurement, once per workload, rather than from
/// [`time_push`]: that runs on every Criterion iteration, and an event there
/// would bury the counters it exists to publish. The workloads are the ones
/// [`push_scope`] times, so the observation describes the same run.
fn observe_push_waits(runtime: &Runtime) {
    for workload in push_workloads() {
        let measurement = expect_bench(
            runtime.block_on(measure_push(workload, 1)),
            "push wait observation failed",
        );
        tracing::info!(
            workload = %workload.label(),
            awaited_capacity_waits = measurement.awaited_capacity_waits,
            capacity_probes = measurement.capacity_probes,
            "push awaited-wait observation"
        );
    }
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
trait BenchLabel {
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
struct BenchScope<W> {
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
fn run_scope<W: BenchLabel + Copy>(
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
fn actor_scope() -> BenchScope<ActorWorkload> {
    BenchScope {
        name: "actor/drain",
        workloads: ACTOR_CANCELLATIONS
            .into_iter()
            .flat_map(|cancellation| {
                ACTOR_FRAME_COUNTS
                    .into_iter()
                    .map(move |frames| ActorWorkload {
                        frames,
                        queue_capacity: ACTOR_QUEUE_CAPACITY,
                        cancellation,
                    })
            })
            .collect(),
        throughput: |workload| Some(elements(workload.frames)),
        measure: time_actor_drain,
    }
}

/// The push scope: the handle's send path.
///
/// Throughput counts sends, which is the work each iteration is asked to
/// perform. Kept as a value alongside the other scopes so the four groups are
/// registered by one path and cannot drift apart in budget or convention.
fn push_scope() -> BenchScope<PushWorkload> {
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
fn startup_scope() -> BenchScope<StartupWorkload> {
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
fn pool_scope() -> BenchScope<PoolWorkload> {
    BenchScope {
        name: "pool/acquisition",
        workloads: uncontended_pool_workloads(),
        throughput: |_workload| None,
        measure: time_pool_acquisition,
    }
}

/// Emit a completed contended sampling run's distribution and counters.
///
/// A single elapsed value was the previous shape of this event, and it was
/// ambiguous: the contended acquire path fails intermittently, so one figure
/// could be the duration of a burst that produced leases or a number for a run
/// that produced none. The distribution separates the two, and the failure
/// count says how much of the run the percentiles do not describe.
///
/// The churn counters are reported per run rather than per sample: they are the
/// pool's cumulative counters read across the sampling window, so a
/// per-burst figure would imply a precision the counters do not have.
///
/// Kept as its own function because a `tracing` macro expands to an
/// `enabled`/not-enabled conditional, and two of them inside one `match` push
/// the caller past the workspace's cognitive-complexity limit.
fn report_contended_success(label: &str, latency: &ContendedLatency) {
    let churn = latency.churn();
    tracing::info!(
        workload = %label,
        burst_timeout = ?CONTENDED_BURST_TIMEOUT,
        attempts = latency.successes() + latency.failure_count(),
        successes = latency.successes(),
        failures = latency.failure_count(),
        scheduler_spawns = churn.scheduler_spawns,
        capacity_waits = churn.capacity_waits,
        min = ?latency.min(),
        median = ?latency.median(),
        p90 = ?latency.p90(),
        max = ?latency.max(),
        mean = ?latency.mean(),
        "contended pool acquisition distribution"
    );
    for failure in latency.failures() {
        report_contended_failure(label, failure);
    }
}

/// Emit the reason a contended burst did not complete as a warning.
fn report_contended_failure(label: &str, error: &str) {
    tracing::warn!(
        workload = %label,
        burst_timeout = ?CONTENDED_BURST_TIMEOUT,
        %error,
        "contended pool acquisition burst did not complete; known acquire-path defect, see \
         docs/runtime-ownership-baselines.md"
    );
}

/// Emit the outcome of one contended sampling run as a structured event.
fn report_contended_outcome(label: &str, outcome: Result<ContendedLatency, String>) {
    match outcome {
        Ok(latency) => report_contended_success(label, &latency),
        Err(error) => report_contended_failure(label, &error),
    }
}

/// Sample contended acquisition as a bounded, untimed observation.
///
/// This deliberately does not register a Criterion group. Sustained contention
/// intermittently stalls the pool acquire path, and Criterion would block during
/// warm-up sampling a scenario that never returns — taking the whole benchmark
/// run with it. The measurement helper converts that stall into a recorded
/// failure after [`CONTENDED_BURST_TIMEOUT`], so the observation always
/// terminates and the failures are reported beside the samples rather than in
/// place of them.
///
/// The churn counters are the issue's task-churn evidence at this scope:
/// whether contended bursts genuinely enter the scheduler's slow path, and what
/// task churn they cost. They are reported per run, beside the distribution
/// rather than as a substitute for it.
fn observe_contended_acquisition(runtime: &Runtime) {
    for workload in contended_pool_workloads() {
        let label = workload.label();
        let outcome = runtime.block_on(measure_contended_latency(
            workload,
            CONTENDED_LATENCY_SAMPLES,
        ));
        report_contended_outcome(&label, outcome);
    }
}

/// Install a subscriber for the benchmark's own events only.
///
/// The contended-acquisition observation reports counters as structured events,
/// and an event with no subscriber is silently discarded. The filter is set to
/// this binary's own target rather than left at a global level: the benchmark
/// drives connection startup, which logs an INFO span per prepared connection,
/// and at 256 connections per iteration that noise buries the counters the
/// observation exists to report. Criterion owns the timing output; `tracing`
/// carries only the counters, which are not timings and must not be read as
/// such.
fn init_observation_subscriber() {
    use tracing_subscriber::filter::LevelFilter;

    tracing_subscriber::fmt()
        .with_target(false)
        .with_max_level(LevelFilter::INFO)
        .with_env_filter(tracing_subscriber::EnvFilter::new(env!("CARGO_CRATE_NAME")))
        .init();
}

/// Entrypoint for the runtime-ownership baseline benchmarks.
fn main() {
    init_observation_subscriber();
    let runtime = expect_bench(Runtime::new(), "benchmark runtime failed");
    let mut criterion = Criterion::default().configure_from_args();

    // One registration path for all four scopes; see `BenchScope`.
    run_scope(&mut criterion, &runtime, actor_scope());
    run_scope(&mut criterion, &runtime, push_scope());
    run_scope(&mut criterion, &runtime, startup_scope());
    run_scope(&mut criterion, &runtime, pool_scope());

    criterion.final_summary();

    observe_push_waits(&runtime);
    observe_contended_acquisition(&runtime);
}
