//! The Criterion groups and the `tracing` events that publish what they count.
//!
//! A count is recorded before its timing pass and emitted under a label whose
//! identity does not depend on it, so Criterion's saved results stay comparable
//! between runs. The contended pool scope is published as an event alone: it
//! cannot survive a Criterion timing pass, but the issue still asks for its
//! number.

use std::{hint::black_box, time::Duration};

use criterion::{BenchmarkId, Criterion};
use tokio::runtime::Runtime;
use wireframe_testing::runtime_ownership_benchmarks::{AcquisitionMode, PoolWorkload};

use crate::{
    pool_bracket::count_pool_acquisition_window,
    probes::{
        ARC_PROBE_ITEMS,
        arc_probes,
        count_window,
        expect_bench,
        infallible,
        timed_infallible,
    },
    scopes::{ALLOCATION_ITERATIONS, allocation_scopes},
};

/// Criterion sample count for every allocation group.
///
/// Lower than Criterion's default because each iteration opens real loopback
/// connections or spawns tasks; the allocation count is already fixed by its
/// own preceding window, so extra samples only tighten the timing.
const SAMPLE_SIZE: usize = 10;

/// Criterion measurement time for every allocation group.
const MEASUREMENT_TIME: Duration = Duration::from_secs(5);

/// Run a workload for `iterations` and sum the durations it reports.
///
/// Used only to give each recorded allocation count a duration beside it in the
/// Criterion report; the count itself came from its own window beforehand.
///
/// The durations are summed rather than a single clock taken around the loop,
/// because a scope may not want its whole replay timed. The pool scope reports
/// only its acquisition rounds, with the pool built and closed outside them; a
/// caller-side clock would silently put the `pool_size` eager connection setups
/// back into the figure the scope exists to report. Summing delegates the
/// boundary to the scope that knows it.
fn time_iterations<F>(operation: &F, iterations: u64) -> Duration
where
    F: Fn() -> Result<Duration, String>,
{
    let mut total = Duration::ZERO;
    for _ in 0..iterations {
        let elapsed = expect_bench(operation(), "allocation timing pass failed");
        total = total.saturating_add(elapsed);
    }
    total
}

/// Contended pool workload, recorded as an observation rather than a timing.
///
/// Contenders must outnumber slots for the pool to be exhausted, and the
/// default `contenders` of 16 clears the default pool size of 4 comfortably.
///
/// This workload is deliberately absent from `allocation_scopes`: sustained
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
pub(crate) fn observe_contended_pool_allocations(runtime: &Runtime) {
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
pub(crate) fn init_observation_subscriber() {
    use tracing_subscriber::filter::LevelFilter;

    tracing_subscriber::fmt()
        .with_target(false)
        .with_max_level(LevelFilter::INFO)
        .with_env_filter(tracing_subscriber::EnvFilter::new(env!("CARGO_CRATE_NAME")))
        .init();
}

/// Record allocation baselines for the actor, push, startup, and pool scopes.
///
/// Each count is taken in its own window, emitted as an event alongside the
/// timing, and registered under a label whose identity does not depend on it.
pub(crate) fn benchmark_allocations(criterion: &mut Criterion, runtime: &Runtime) {
    let mut group = criterion.benchmark_group("runtime_ownership/allocations");
    group
        .sample_size(SAMPLE_SIZE)
        .measurement_time(MEASUREMENT_TIME);

    for scope in allocation_scopes(runtime) {
        let label = scope.label.clone();
        let allocations = expect_bench((scope.count)(), &format!("{label} allocation window"));
        report_allocation_baseline(&label, allocations);
        let (_, replay) = scope.into_replay();

        group.bench_function(BenchmarkId::from_parameter(&label), |bencher| {
            bencher.iter_custom(|iterations| {
                black_box(allocations);
                time_iterations(&replay, iterations)
            });
        });
    }

    group.finish();
}

/// Emit one scope's allocation count as a structured event.
///
/// The count belongs beside the benchmark, not inside its ID. Criterion keys
/// saved results, `--baseline` comparisons and change reports on the ID, and
/// these counts move by tens between runs, so an ID carrying one would make
/// every run a *new* benchmark: never a change against the last, and a fresh
/// directory under `target/criterion` each time. A stable label keeps the
/// comparison, and this event publishes the count that the comparison cannot
/// carry.
///
/// Kept as its own function because a `tracing` macro expands to an
/// `enabled`/not-enabled conditional, and the two registration sites each push
/// their loop past the workspace's cognitive-complexity limit with one inline.
fn report_allocation_baseline(label: &str, allocations: usize) {
    tracing::info!(
        workload = %label,
        iterations = ALLOCATION_ITERATIONS,
        allocations,
        "allocation baseline"
    );
}

/// Record the nested-`Arc` sensitivity pair as labelled counts.
///
/// The point of this group is the recorded contrast: two counts whose difference
/// is the per-item allocation the harness must resolve.
/// `verify_allocation_harness` has already asserted that difference, so a green
/// run here is evidence the counter works rather than merely a pair of numbers.
pub(crate) fn benchmark_arc_sensitivity(criterion: &mut Criterion) {
    let mut group = criterion.benchmark_group("runtime_ownership/arc_sensitivity");
    group
        .sample_size(SAMPLE_SIZE)
        .measurement_time(MEASUREMENT_TIME);

    for (mode, probe) in arc_probes() {
        let operation = infallible(move || probe(ARC_PROBE_ITEMS));
        let allocations = count_window(mode, &operation);
        // The timing pass needs a closure that reports its own duration, so the
        // shared operation's infallible body is wrapped in a timed adapter rather
        // than closing the clock around `time_iterations`'s whole loop.
        let replay = timed_infallible(move || probe(ARC_PROBE_ITEMS));
        let labelled = format!("arc_{mode}_items_{ARC_PROBE_ITEMS}");
        report_allocation_baseline(&labelled, allocations);

        group.bench_function(BenchmarkId::from_parameter(labelled), |bencher| {
            bencher.iter_custom(|iterations| {
                black_box(allocations);
                time_iterations(&replay, iterations)
            });
        });
    }

    group.finish();
}
