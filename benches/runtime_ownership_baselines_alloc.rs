//! Criterion benchmarks recording allocation baselines for the runtime-ownership
//! hot paths.
//!
//! This binary is separate from `runtime_ownership_baselines` because
//! `#[global_allocator]` is per-process: installing the counting allocator would
//! tax every timing sample in the sibling binary. Both binaries drive the same
//! workload helpers in [`wireframe_testing::runtime_ownership_benchmarks`].
//!
//! Each baseline is embedded in its benchmark label as `..._allocs_<n>`, so a
//! Criterion report carries the count next to the timing and no separate log
//! needs to be kept.
//!
//! # What the number means
//!
//! The recorded value is a **process-global allocation delta** across the
//! measurement window. It includes the async runtime's own allocations, task
//! setup, closure captures, and anything else that ran in the window — not only
//! the code under test. Compare counts across runs of this same binary; do not
//! read them as a per-operation cost.
//!
//! # The sensitivity experiment
//!
//! Acceptance criterion 5 asks the harness to identify whether removing a
//! nested `Arc` changes the allocation count. That question has two possible
//! answers, and the harness must be able to tell them apart rather than assume
//! either:
//!
//! - cloning a shared `Arc` costs an atomic increment and **no** allocation, so removing such a
//!   clone changes nothing an allocation counter can see; while
//! - constructing a fresh `Arc` per item costs **one allocation per item**, so removing it moves
//!   the count by at least the item count.
//!
//! [`verify_allocation_harness`] runs both shapes before any measurement and
//! fails loudly unless the difference is at least one allocation per item, which
//! can only hold if the counter actually resolves the per-item allocation. The
//! recorded counts are transcribed into `docs/runtime-ownership-baselines.md`.
//!
//! # Contended pool acquisition
//!
//! Contended pool acquisition is counted once, after `Criterion::final_summary`,
//! and reported as a `tracing` event rather than timed. Sustained contention
//! intermittently fails or stalls the pool acquire path — a pre-existing defect
//! recorded in `docs/runtime-ownership-baselines.md` and belonging to ADR 013 /
//! issue #647. Criterion's timing pass runs far more iterations than an
//! allocation window, so it reaches that interleaving almost every run; that is
//! how this binary failed on its first execution. The count itself is unaffected
//! because it comes from a single counted window.

use std::{
    hint::black_box,
    sync::Arc,
    time::{Duration, Instant},
};

use criterion::{BenchmarkId, Criterion};
use tokio::runtime::Runtime;
use wireframe_testing::{
    benchmark_alloc_support::{CountingAllocator, count_allocations},
    runtime_ownership_benchmarks::{
        AcquisitionMode,
        ActorWorkload,
        CancellationMode,
        PoolHarness,
        PoolWorkload,
        PushScenario,
        PushWorkload,
        StartupPath,
        StartupWorkload,
        measure_actor_drain,
        measure_push,
        measure_startup,
    },
};

/// Process allocator for this benchmark binary.
#[global_allocator]
static GLOBAL_ALLOCATOR: CountingAllocator = CountingAllocator;

/// Criterion sample count for every allocation group.
///
/// Lower than Criterion's default because each iteration opens real loopback
/// connections or spawns tasks; the allocation count is already fixed by its
/// own preceding window, so extra samples only tighten the timing.
const SAMPLE_SIZE: usize = 10;

/// Criterion measurement time for every allocation group.
const MEASUREMENT_TIME: Duration = Duration::from_secs(5);

/// Workload iterations performed inside each allocation window.
///
/// Small on purpose: the window's count grows with the iteration count, and a
/// large one would make every label a four-digit number that is harder to
/// compare at a glance.
const ALLOCATION_ITERATIONS: u64 = 4;

/// Frames per iteration in the actor allocation workload.
const ACTOR_FRAMES: usize = 256;

/// Per-queue capacity in the actor allocation workload.
///
/// At least half [`ACTOR_FRAMES`], so the workload fits its queues and does not
/// block on back-pressure.
const ACTOR_QUEUE_CAPACITY: usize = 128;

/// Sends per iteration in the push allocation workloads.
const PUSH_SENDS: usize = 256;

/// Queue capacity for the drained push allocation workload.
///
/// Deliberately under [`PUSH_SENDS`], so the producer can outrun its consumer.
/// Whether it did is reported through the wait counters rather than asserted;
/// the window is about allocation, which a park only adds to.
const PUSH_QUEUE_CAPACITY: usize = 64;

/// Queue capacity for the overflow push allocation workload.
const PUSH_OVERFLOW_CAPACITY: usize = 8;

/// Items per probe in the nested-`Arc` sensitivity experiment.
const ARC_PROBE_ITEMS: usize = 256;

/// A labelled workload, the closure that counts it, and the closure that replays
/// it.
///
/// Counting and replaying are separate closures rather than one, because most
/// scopes count by wrapping their workload in a counting window while the pool
/// scope must count something narrower than it replays — see
/// [`count_pool_acquisition_window`]. Splitting them is what lets a scope whose
/// bracket is not "wrap the whole workload" still take part in the same list.
///
/// Both closures are boxed so scopes with different captured workloads share one
/// list type, which keeps the allocation and timing groups iterating one set.
struct AllocationScope<'a> {
    /// Label carried into the benchmark id, naming the workload's parameters.
    label: String,
    /// Take this scope's allocation count, in a fresh counting window.
    ///
    /// Called once per scope, before the timing passes. It is a closure rather
    /// than a plain number so each scope owns its bracketing, and it returns a
    /// `Result` so a scope that cannot be counted stops the run rather than
    /// recording a fabricated zero.
    count: Box<dyn Fn() -> Result<usize, String> + 'a>,
    /// Replay the same workload without counting, for the timing passes.
    replay: Box<dyn Fn() -> Result<(), String> + 'a>,
}

impl<'a> AllocationScope<'a> {
    /// A scope counted by wrapping its whole workload in a counting window.
    ///
    /// The common case: the workload has no region whose cost needs excluding, so
    /// counting it and replaying it are the same call. One operation is shared by
    /// both closures rather than written twice, so the count and the timing
    /// cannot drift apart onto different workloads — the failure that writing
    /// the call out twice invites.
    fn windowed(label: String, operation: impl Fn() -> Result<(), String> + 'a) -> Self {
        // An `Arc` rather than a clone bound: the closure's captures are not
        // necessarily `Clone`, and sharing one instance is what guarantees both
        // closures run the same workload.
        let shared = Arc::new(operation);
        let counted = Arc::clone(&shared);
        Self {
            label,
            count: Box::new(move || count_allocations(|| (counted)())),
            replay: Box::new(move || (shared)()),
        }
    }

    /// Box this scope's replay closure for the timing pass.
    fn into_replay(self) -> (String, Box<dyn Fn() -> Result<(), String> + 'a>) {
        (self.label, self.replay)
    }
}

/// A named allocation probe: a label, and the function that allocates.
///
/// Named rather than inlined in [`arc_probes`]'s return type because the bare
/// pair of a `&'static str` and a function pointer exceeds the workspace's
/// type-complexity limit, and because the alias documents what the two halves
/// mean together.
///
/// The probe is infallible. Returning `Result` here would be dead machinery —
/// neither probe has a failure mode — and infallibility is what lets the two
/// probe functions stay plain loops instead of error-returning shapes they do
/// not need.
type ArcProbe = (&'static str, fn(usize));

/// Unwrap a benchmark helper's result, terminating with a diagnostic on error.
///
/// A failed workload cannot produce a meaningful sample, so stopping the
/// benchmark is the only honest outcome: recording a number for a scenario that
/// did not run would put a fabricated baseline on record.
fn expect_bench<T, E: std::fmt::Display>(result: Result<T, E>, context: &str) -> T {
    match result {
        Ok(value) => value,
        Err(error) => panic!("{context}: {error}"),
    }
}

/// Run one workload inside an allocation-counting window, returning the delta.
fn count_window<F>(context: &str, operation: F) -> usize
where
    F: FnOnce() -> Result<(), String>,
{
    expect_bench(count_allocations(operation), context)
}

/// Adapt an infallible probe to the fallible shape the counting window expects.
///
/// The nested-`Arc` probes have no failure mode, so they stay plain closures and
/// this supplies the `Result` the window expects. The alternative — making every
/// probe return `Ok(())` on a path that can never be `Err` — would be dead
/// machinery the compiler rightly flags.
fn infallible(probe: impl Fn()) -> impl Fn() -> Result<(), String> {
    move || {
        probe();
        Ok(())
    }
}

/// Run a workload for `iterations` and report how long that took.
///
/// Used only to give each recorded allocation count a duration beside it in the
/// Criterion report; the count itself came from its own window beforehand.
fn time_iterations<F>(operation: &F, iterations: u64) -> Duration
where
    F: Fn() -> Result<(), String>,
{
    let started = Instant::now();
    for _ in 0..iterations {
        expect_bench(operation(), "allocation timing pass failed");
    }
    started.elapsed()
}

/// Contended pool workload, recorded as an observation rather than a timing.
///
/// Contenders must outnumber slots for the pool to be exhausted, and the
/// default `contenders` of 16 clears the default pool size of 4 comfortably.
///
/// This workload is deliberately absent from [`allocation_scopes`]: sustained
/// contention intermittently stalls the pool acquire path or fails it with a
/// spurious `disconnected`, and Criterion's timing pass runs far more
/// iterations than the four in an allocation window, so it reaches that
/// interleaving almost every run. Measuring it here once, under a counted
/// window, records the number the issue asks for without handing the scenario
/// to a pass that cannot survive it. See `docs/runtime-ownership-baselines.md`.
fn contended_pool_workload() -> PoolWorkload {
    PoolWorkload {
        pool_size: 4,
        mode: AcquisitionMode::ContendedFifo,
        ..PoolWorkload::default()
    }
}

/// Drive a measurement future to completion, discarding its counters.
///
/// The counters are already carried in the benchmark label, so dropping them
/// here loses nothing: a scope closure's only job is to perform the work the
/// counting window observes. Adapting every scope through one helper is what
/// keeps [`allocation_scopes`] short enough to read and keeps the six scopes
/// visibly identical in shape.
fn run_measurement<T, E: std::fmt::Display>(
    runtime: &Runtime,
    measurement: impl std::future::Future<Output = Result<T, E>>,
) -> Result<(), String> {
    runtime
        .block_on(measurement)
        .map(|_| ())
        .map_err(|error| error.to_string())
}

/// The workload parameters this binary records allocations for, by scope.
///
/// Split from [`allocation_scopes`] because the six literal constructions alone
/// push that function past the workspace's line limit. Keeping the parameters in
/// one block also makes them easy to compare at a glance, which is their point:
/// each differs from the timing binary's defaults deliberately, to keep the
/// allocation windows short enough to compare as small integers.
struct AllocationWorkloads {
    /// Actor drain, with cancellation disabled so the window covers the loop.
    actor: ActorWorkload,
    /// Awaited send into a draining queue that may not keep up.
    push_drained: PushWorkload,
    /// Non-blocking send against a deliberately small, full queue.
    push_overflow: PushWorkload,
    /// The shared prepared root, which prepares once for all connections.
    startup_prepared: StartupWorkload,
    /// The per-connection factory path, which prepares once per connection.
    startup_per_connection: StartupWorkload,
    /// Uncontended acquire/release, the only pool mode safe to time.
    pool: PoolWorkload,
}

impl AllocationWorkloads {
    /// Build the six scopes at their allocation-window parameters.
    fn new() -> Self {
        Self {
            actor: ActorWorkload {
                frames: ACTOR_FRAMES,
                queue_capacity: ACTOR_QUEUE_CAPACITY,
                cancellation: CancellationMode::Disabled,
            },
            push_drained: PushWorkload {
                // Drained, not uncontended: at this send count the queue cannot
                // hold the run, so the previous `UncontendedHigh` label claimed a
                // no-back-pressure property the workload could not support.
                scenario: PushScenario::DrainedHigh,
                queue_capacity: PUSH_QUEUE_CAPACITY,
                sends: PUSH_SENDS,
            },
            push_overflow: PushWorkload {
                scenario: PushScenario::OverflowDrop,
                queue_capacity: PUSH_OVERFLOW_CAPACITY,
                sends: PUSH_SENDS,
            },
            startup_prepared: StartupWorkload {
                path: StartupPath::PreparedRoot,
                ..StartupWorkload::default()
            },
            startup_per_connection: StartupWorkload {
                path: StartupPath::PerConnectionFactory,
                ..StartupWorkload::default()
            },
            pool: PoolWorkload {
                pool_size: 4,
                mode: AcquisitionMode::Uncontended,
                ..PoolWorkload::default()
            },
        }
    }
}

/// Build the runtime-ownership workloads this binary records allocations for.
///
/// One list feeds both the allocation and timing groups, so adding a scope can
/// not silently skip the timing side. The contended pool scope is excluded
/// because it cannot be timed at all; see [`contended_pool_workload`] and
/// [`observe_contended_pool_allocations`].
fn allocation_scopes(runtime: &Runtime) -> Vec<AllocationScope<'_>> {
    let workloads = AllocationWorkloads::new();
    let iterations = ALLOCATION_ITERATIONS;

    vec![
        AllocationScope::windowed(workloads.actor.label(), move || {
            run_measurement(runtime, measure_actor_drain(workloads.actor, iterations))
        }),
        AllocationScope::windowed(workloads.push_drained.label(), move || {
            run_measurement(runtime, measure_push(workloads.push_drained, iterations))
        }),
        AllocationScope::windowed(workloads.push_overflow.label(), move || {
            run_measurement(runtime, measure_push(workloads.push_overflow, iterations))
        }),
        AllocationScope::windowed(workloads.startup_prepared.label(), move || {
            run_measurement(
                runtime,
                measure_startup(workloads.startup_prepared, iterations),
            )
        }),
        AllocationScope::windowed(workloads.startup_per_connection.label(), move || {
            run_measurement(
                runtime,
                measure_startup(workloads.startup_per_connection, iterations),
            )
        }),
        AllocationScope {
            label: workloads.pool.label(),
            count: Box::new(move || {
                count_pool_acquisition_window(runtime, workloads.pool, iterations)
            }),
            replay: Box::new(move || run_pool_acquisition(runtime, workloads.pool, iterations)),
        },
    ]
}

/// Run the pool scope the way the timing pass should replay it.
///
/// One call builds a pool, measures `iterations` acquire/release rounds against
/// it, and closes it — the same workload the count beside it describes, so the
/// timing and the count are of one thing.
///
/// # Errors
///
/// Returns a description of the construction, warm-up, or acquisition failure.
fn run_pool_acquisition(
    runtime: &Runtime,
    workload: PoolWorkload,
    iterations: u64,
) -> Result<(), String> {
    let harness = runtime.block_on(PoolHarness::build(workload))?;
    let measured = runtime.block_on(harness.measure(iterations));
    runtime.block_on(harness.close());
    measured.map(|_| ())
}

/// Count the allocations of one pool scope, with construction outside the window.
///
/// [`run_pool_acquisition`] builds its pool inside the call, so counting a
/// closure that runs it directly charges the window for pool construction. `bb8`
/// connects eagerly, so that is one allocation-bearing loopback connection per
/// slot — a count that scales with `pool_size` and swamps the acquire path the
/// window exists to observe. Building and closing the harness *around* the
/// counted call is what separates them, and the ordering is why this helper
/// exists rather than a comment on the call site.
///
/// Construction cost is not discarded, only excluded: `PoolMeasurement` carries
/// it on its `construction` field, so a caller that wants the figure can read
/// it. The timing binary does not currently print it, since its `elapsed`
/// deliberately excludes construction and it reads only the counters beside it.
///
/// # Errors
///
/// Returns a description of a construction, warm-up, or acquisition failure, or
/// of a harness that could not be built.
fn count_pool_acquisition_window(
    runtime: &Runtime,
    workload: PoolWorkload,
    iterations: u64,
) -> Result<usize, String> {
    // Outside the window: construction and warm-up.
    let harness = runtime.block_on(PoolHarness::build(workload))?;
    let counted = count_allocations(|| runtime.block_on(harness.measure(iterations)).map(|_| ()));
    // Outside the window: teardown.
    runtime.block_on(harness.close());
    counted
}

/// Emit a successful contended allocation count as an event.
///
/// Kept as its own function because a `tracing` macro expands to an
/// `enabled`/not-enabled conditional, and two of them inside one `match` push
/// the caller past the workspace's cognitive-complexity limit.
fn report_contended_success(label: &str, allocations: usize) {
    tracing::info!(
        workload = %label,
        iterations = ALLOCATION_ITERATIONS,
        allocations,
        "contended pool allocation observation"
    );
}

/// Emit a failed contended allocation count as a warning.
fn report_contended_failure(label: &str, error: &str) {
    tracing::warn!(
        workload = %label,
        iterations = ALLOCATION_ITERATIONS,
        %error,
        "contended pool allocations could not be counted; known acquire-path defect, see \
         docs/runtime-ownership-baselines.md"
    );
}

/// Emit one contended allocation count, or the reason it could not be taken.
fn report_contended_allocations(label: &str, outcome: Result<usize, String>) {
    match outcome {
        Ok(allocations) => report_contended_success(label, allocations),
        Err(error) => report_contended_failure(label, &error),
    }
}

/// Record the contended pool allocation count as a bounded observation.
///
/// Run once after Criterion has finished, so a failure is reported rather than
/// aborting the benchmark run. The count comes from the same bracketed window the
/// uncontended pool scope uses, so the two are comparable; only the repetition
/// and the timing are dropped, because repetition is what the acquire-path defect
/// makes unreliable. See `docs/runtime-ownership-baselines.md`.
fn observe_contended_pool_allocations(runtime: &Runtime) {
    let workload = contended_pool_workload();
    let label = workload.label();
    let outcome = count_pool_acquisition_window(runtime, workload, ALLOCATION_ITERATIONS);

    report_contended_allocations(&label, outcome);
}

/// Install a subscriber for the allocation binary's own events only.
///
/// The contended observation reports a count as a structured event, and an
/// event with no subscriber is silently discarded. The filter is set to this
/// binary's own target rather than left at a global level, so the connection
/// startup workloads' per-connection INFO spans do not bury it. Criterion owns
/// the timing output; `tracing` carries only the count.
fn init_observation_subscriber() {
    use tracing_subscriber::filter::LevelFilter;

    tracing_subscriber::fmt()
        .with_target(false)
        .with_max_level(LevelFilter::INFO)
        .with_env_filter(tracing_subscriber::EnvFilter::new(env!("CARGO_CRATE_NAME")))
        .init();
}

/// Clone one shared `Arc` per item, against one freshly allocated shared value.
///
/// A clone is an atomic increment of the strong count: it moves no allocation,
/// which is one half of what the sensitivity experiment must resolve. The lone
/// `Arc::new` *does* allocate — one control block, no matter how many items are
/// cloned — so the expected delta against [`probe_fresh_allocation`] is
/// `items - 1`, not `items`. The assertion in [`verify_allocation_harness`]
/// states that arithmetic explicitly.
fn probe_shared_clone(items: usize) {
    let shared = Arc::new(black_box(0u8));
    for _ in 0..items {
        black_box(Arc::clone(&shared));
    }
}

/// Allocate one fresh `Arc` per item.
///
/// Each construction heap-allocates its control block, so this is the shape
/// whose removal an allocation counter must be able to see.
fn probe_fresh_allocation(items: usize) {
    for _ in 0..items {
        black_box(Arc::new(black_box(0u8)));
    }
}

/// Prove the allocation harness resolves what the runtime-ownership baselines
/// depend on, stopping the binary loudly if it cannot.
///
/// Runs before any runtime or Criterion harness exists, because the counter is
/// process-global: a background thread allocating during a window would make the
/// zero case untrustworthy. Three checks:
///
/// 1. a closure that allocates nothing reports exactly zero, catching a gate stuck open;
/// 2. a closure that allocates a known buffer reports a positive count, catching a gate that never
///    opens — which would otherwise record a silent baseline of zeros for every scope; and
/// 3. a per-item `Arc` construction reports one allocation per item more than cloning a shared
///    `Arc`, which is the resolution acceptance criterion 5 requires.
fn verify_allocation_harness() {
    let zero = count_window("zero-allocation probe", || Ok(()));
    assert_eq!(
        zero, 0,
        "an allocation-free closure must report zero; a non-zero count means the counting gate is \
         stuck open or something else ran during the window"
    );

    let positive = count_window("positive-allocation probe", || {
        let buffer: Vec<u8> = vec![0; 1024];
        black_box(buffer.len());
        Ok(())
    });
    assert!(
        positive > 0,
        "allocating a 1024-byte buffer must be observed; a zero count means the counting gate \
         never opens and every baseline below would be a silent zero"
    );

    let shared = count_window(
        "shared-clone probe",
        infallible(|| probe_shared_clone(ARC_PROBE_ITEMS)),
    );
    let fresh = count_window(
        "fresh-allocation probe",
        infallible(|| probe_fresh_allocation(ARC_PROBE_ITEMS)),
    );
    // The shared probe allocates once for its single `Arc::new`, and every clone
    // after that is a bare atomic increment, so the per-item difference is
    // `ARC_PROBE_ITEMS - 1` at minimum. Demanding the full item count would fail
    // on a correct counter for an off-by-one reason.
    let expected_delta = ARC_PROBE_ITEMS.saturating_sub(1);
    assert!(
        fresh >= shared.saturating_add(expected_delta),
        "per-item `Arc` construction must cost an allocation per item beyond the shared probe's \
         single allocation: {fresh} fresh vs {shared} shared across {ARC_PROBE_ITEMS} items, \
         expecting a delta of at least {expected_delta}"
    );
}

/// The two nested-`Arc` probe shapes, paired with their labels.
fn arc_probes() -> Vec<ArcProbe> {
    vec![
        ("shared_clone", probe_shared_clone),
        ("fresh_per_item", probe_fresh_allocation),
    ]
}

/// Prove the pool scope's bracket really excludes pool construction.
///
/// The pool scope is the one place the count is *not* a plain wrap of its
/// workload: [`count_pool_acquisition_window`] builds the pool around the counted
/// region rather than inside it. That is a claim in a doc comment, and the
/// doc comment is the only thing asserting it — the count itself is a bare
/// integer that looks the same whether the bracket is right or wrong. This check
/// makes the claim falsifiable by measuring both sides.
///
/// The two are compared at `iterations = 0`, which is what makes the comparison
/// decisive rather than statistical. At zero iterations the measured phase
/// contributes nothing and construction is the only thing left that allocates,
/// so the bracketed window must read exactly zero. Comparing full runs instead —
/// some 2 300 allocations of which construction is 50 — puts a 2% effect against
/// run-to-run noise of several hundred, and measurement showed the two
/// distributions overlapping almost completely. Erasing the measured phase
/// removes the noise rather than trying to see past it.
///
/// Both numbers come from the same counting machinery and the same workload;
/// only the bracketing differs. A third reading at the normal iteration count
/// closes the other half of the claim: the equality above is satisfied both by
/// a correct bracket and by one that wraps nothing, and only the third reading
/// can tell those apart.
fn verify_pool_bracket_excludes_construction(runtime: &Runtime) {
    let workload = AllocationWorkloads::new().pool;

    let (empty_bracketed, empty_whole) = expect_bench(
        pool_bracket_readings(runtime, workload, 0),
        "zero-iteration pool allocation window failed",
    );

    // Without this the equality below is vacuous: a counter that observed
    // nothing would read zero on both sides and the bracket would look perfect.
    // Construction is real and countable, so the unbracketed window must see it.
    assert!(
        empty_whole > 0,
        "a zero-iteration pool acquisition must still count the pool's construction in the \
         unbracketed window, but it counted {empty_whole} allocations for a {}-slot pool. The \
         allocation counter is not observing this workload, so the bracket equality below proves \
         nothing",
        workload.pool_size
    );
    assert_eq!(
        empty_bracketed, 0,
        "pool construction belongs outside the bracketed allocation window, but a zero-iteration \
         window counted {empty_bracketed} of the {empty_whole} allocations that construction \
         costs a {}-slot pool. Construction has been pulled back inside the window, so every \
         published pool allocation figure reports connection setup as acquire-path cost",
        workload.pool_size
    );

    // The other half of the claim: the window must contain the acquire path, not
    // merely exclude construction. A bracket that wrapped nothing at all would
    // satisfy the equality above just as well.
    let (measured_bracketed, _) = expect_bench(
        pool_bracket_readings(runtime, workload, ALLOCATION_ITERATIONS),
        "measured pool allocation window failed",
    );
    assert!(
        measured_bracketed > 0,
        "the bracketed window must count the acquire path's allocations, but \
         {ALLOCATION_ITERATIONS} iterations of a {}-slot pool counted {measured_bracketed}. The \
         window no longer wraps the measured phase",
        workload.pool_size
    );
}

/// Take one reading from each window over the same workload.
///
/// Returns `(bracketed, whole)`: the acquisition workload counted with the
/// pool's construction outside the window, and with it inside. Keeping both
/// readings in one place is what lets a caller compare them without the two
/// windows silently drifting onto different workloads.
///
/// # Errors
///
/// Returns the first window's failure description.
fn pool_bracket_readings(
    runtime: &Runtime,
    workload: PoolWorkload,
    iterations: u64,
) -> Result<(usize, usize), String> {
    let bracketed = count_pool_acquisition_window(runtime, workload, iterations)?;
    let whole = count_allocations(|| run_pool_acquisition(runtime, workload, iterations))?;
    Ok((bracketed, whole))
}

/// Record allocation baselines for the actor, push, startup, and pool scopes.
///
/// Each count is taken in its own window and written into the benchmark label,
/// so a Criterion report carries the baseline beside the timing.
fn benchmark_allocations(criterion: &mut Criterion, runtime: &Runtime) {
    let mut group = criterion.benchmark_group("runtime_ownership/allocations");
    group
        .sample_size(SAMPLE_SIZE)
        .measurement_time(MEASUREMENT_TIME);

    for scope in allocation_scopes(runtime) {
        let label = scope.label.clone();
        let allocations = expect_bench((scope.count)(), &format!("{label} allocation window"));
        let labelled = format!("{label}_allocs_{allocations}");
        let (_, replay) = scope.into_replay();

        group.bench_function(BenchmarkId::from_parameter(labelled), |bencher| {
            bencher.iter_custom(|iterations| {
                black_box(allocations);
                time_iterations(&replay, iterations)
            });
        });
    }

    group.finish();
}

/// Record the nested-`Arc` sensitivity pair as labelled counts.
///
/// The point of this group is the recorded contrast: two counts whose difference
/// is the per-item allocation the harness must resolve.
/// [`verify_allocation_harness`] has already asserted that difference, so a green
/// run here is evidence the counter works rather than merely a pair of numbers.
fn benchmark_arc_sensitivity(criterion: &mut Criterion) {
    let mut group = criterion.benchmark_group("runtime_ownership/arc_sensitivity");
    group
        .sample_size(SAMPLE_SIZE)
        .measurement_time(MEASUREMENT_TIME);

    for (mode, probe) in arc_probes() {
        let operation = infallible(move || probe(ARC_PROBE_ITEMS));
        let allocations = count_window(mode, &operation);
        let labelled = format!("arc_{mode}_items_{ARC_PROBE_ITEMS}_allocs_{allocations}");

        group.bench_function(BenchmarkId::from_parameter(labelled), |bencher| {
            bencher.iter_custom(|iterations| {
                black_box(allocations);
                time_iterations(&operation, iterations)
            });
        });
    }

    group.finish();
}

/// Entrypoint for the runtime-ownership allocation baselines.
fn main() {
    init_observation_subscriber();

    // The harness check must precede the runtime and Criterion: both can start
    // threads whose allocations would otherwise land inside a measurement window.
    verify_allocation_harness();

    let runtime = expect_bench(Runtime::new(), "allocation benchmark runtime failed");
    // Needs the runtime to build a real pool, so it follows construction rather
    // than preceding it like `verify_allocation_harness`. Both run before
    // Criterion, whose own threads would otherwise allocate inside a window.
    verify_pool_bracket_excludes_construction(&runtime);

    let mut criterion = Criterion::default().configure_from_args();

    benchmark_allocations(&mut criterion, &runtime);
    benchmark_arc_sensitivity(&mut criterion);

    criterion.final_summary();

    observe_contended_pool_allocations(&runtime);
}
