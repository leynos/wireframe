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
        PoolWorkload,
        PushScenario,
        PushWorkload,
        StartupPath,
        StartupWorkload,
        measure_actor_drain,
        measure_pool_acquisition,
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

/// Queue capacity for the uncontended push allocation workload.
const PUSH_QUEUE_CAPACITY: usize = 64;

/// Queue capacity for the overflow push allocation workload.
const PUSH_OVERFLOW_CAPACITY: usize = 8;

/// Items per probe in the nested-`Arc` sensitivity experiment.
const ARC_PROBE_ITEMS: usize = 256;

/// A labelled workload plus the closure that runs one iteration of it.
///
/// The closure is boxed so scopes with different captured workloads share one
/// list type, which keeps the allocation and timing groups iterating the same
/// set.
type AllocationScope<'a> = (String, Box<dyn Fn() -> Result<(), String> + 'a>);

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

/// Build the runtime-ownership workloads this binary records allocations for.
///
/// One list feeds both the allocation and timing groups, so adding a scope can
/// not silently skip the timing side.
fn allocation_scopes(runtime: &Runtime) -> Vec<AllocationScope<'_>> {
    let actor = ActorWorkload {
        frames: ACTOR_FRAMES,
        queue_capacity: ACTOR_QUEUE_CAPACITY,
        cancellation: CancellationMode::Disabled,
    };
    let push_uncontended = PushWorkload {
        scenario: PushScenario::UncontendedHigh,
        queue_capacity: PUSH_QUEUE_CAPACITY,
        sends: PUSH_SENDS,
    };
    let push_overflow = PushWorkload {
        scenario: PushScenario::OverflowDrop,
        queue_capacity: PUSH_OVERFLOW_CAPACITY,
        sends: PUSH_SENDS,
    };
    let startup_prepared = StartupWorkload {
        path: StartupPath::PreparedRoot,
        ..StartupWorkload::default()
    };
    let startup_per_connection = StartupWorkload {
        path: StartupPath::PerConnectionFactory,
        ..StartupWorkload::default()
    };
    let pool_uncontended = PoolWorkload {
        pool_size: 4,
        mode: AcquisitionMode::Uncontended,
        ..PoolWorkload::default()
    };
    let pool_contended = PoolWorkload {
        pool_size: 4,
        mode: AcquisitionMode::ContendedFifo,
        ..PoolWorkload::default()
    };

    vec![
        (
            actor.label(),
            Box::new(move || {
                runtime
                    .block_on(measure_actor_drain(actor, ALLOCATION_ITERATIONS))
                    .map(|_| ())
            }),
        ),
        (
            push_uncontended.label(),
            Box::new(move || {
                runtime
                    .block_on(measure_push(push_uncontended, ALLOCATION_ITERATIONS))
                    .map(|_| ())
            }),
        ),
        (
            push_overflow.label(),
            Box::new(move || {
                runtime
                    .block_on(measure_push(push_overflow, ALLOCATION_ITERATIONS))
                    .map(|_| ())
            }),
        ),
        (
            startup_prepared.label(),
            Box::new(move || {
                runtime
                    .block_on(measure_startup(startup_prepared, ALLOCATION_ITERATIONS))
                    .map(|_| ())
            }),
        ),
        (
            startup_per_connection.label(),
            Box::new(move || {
                runtime
                    .block_on(measure_startup(
                        startup_per_connection,
                        ALLOCATION_ITERATIONS,
                    ))
                    .map(|_| ())
            }),
        ),
        (
            pool_uncontended.label(),
            Box::new(move || {
                runtime
                    .block_on(measure_pool_acquisition(
                        pool_uncontended,
                        ALLOCATION_ITERATIONS,
                    ))
                    .map(|_| ())
            }),
        ),
        (
            pool_contended.label(),
            Box::new(move || {
                runtime
                    .block_on(measure_pool_acquisition(
                        pool_contended,
                        ALLOCATION_ITERATIONS,
                    ))
                    .map(|_| ())
            }),
        ),
    ]
}

/// Clone one shared `Arc` per item, against one freshly allocated shared value.
///
/// A clone is an atomic increment of the strong count: it moves no allocation,
/// which is one half of what the sensitivity experiment must resolve. The lone
/// `Arc::new` *does* allocate — one control block, no matter how many items are
/// cloned — so the expected delta against [`probe_fresh_allocation`] is
/// `items - 1`, not `items`. The assertion in [`verify_allocation_harness`]
/// states that arithmetic explicitly.
fn probe_shared_clone(items: usize) -> Result<(), String> {
    let shared = Arc::new(black_box(0u8));
    for _ in 0..items {
        black_box(Arc::clone(&shared));
    }
    Ok(())
}

/// Allocate one fresh `Arc` per item.
///
/// Each construction heap-allocates its control block, so this is the shape
/// whose removal an allocation counter must be able to see.
fn probe_fresh_allocation(items: usize) -> Result<(), String> {
    for _ in 0..items {
        black_box(Arc::new(black_box(0u8)));
    }
    Ok(())
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

    let shared = count_window("shared-clone probe", || probe_shared_clone(ARC_PROBE_ITEMS));
    let fresh = count_window("fresh-allocation probe", || {
        probe_fresh_allocation(ARC_PROBE_ITEMS)
    });
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
fn arc_probes() -> Vec<(&'static str, fn(usize) -> Result<(), String>)> {
    vec![
        ("shared_clone", probe_shared_clone),
        ("fresh_per_item", probe_fresh_allocation),
    ]
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

    for (label, operation) in allocation_scopes(runtime) {
        let allocations = count_window(&label, || operation());
        let labelled = format!("{label}_allocs_{allocations}");

        group.bench_function(BenchmarkId::from_parameter(labelled), |bencher| {
            bencher.iter_custom(|iterations| {
                black_box(allocations);
                time_iterations(&operation, iterations)
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
        let allocations = count_window(mode, || probe(ARC_PROBE_ITEMS));
        let labelled = format!("arc_{mode}_items_{ARC_PROBE_ITEMS}_allocs_{allocations}");

        group.bench_function(BenchmarkId::from_parameter(labelled), |bencher| {
            bencher.iter_custom(|iterations| {
                black_box(allocations);
                time_iterations(&|| probe(ARC_PROBE_ITEMS), iterations)
            });
        });
    }

    group.finish();
}

/// Entrypoint for the runtime-ownership allocation baselines.
fn main() {
    // The harness check must precede the runtime and Criterion: both can start
    // threads whose allocations would otherwise land inside a measurement window.
    verify_allocation_harness();

    let runtime = expect_bench(Runtime::new(), "allocation benchmark runtime failed");
    let mut criterion = Criterion::default().configure_from_args();

    benchmark_allocations(&mut criterion, &runtime);
    benchmark_arc_sensitivity(&mut criterion);

    criterion.final_summary();
}
