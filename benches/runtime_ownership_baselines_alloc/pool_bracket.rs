//! The pool scope's bracket, and the checks that make it falsifiable.
//!
//! The pool scope is the one place the count is *not* a plain wrap of its
//! workload: the pool is built around the counted region rather than inside it.
//! That claim needs both a construction-excluding count and evidence that the
//! exclusion is real, which is what the two verification halves below supply.

use std::time::{Duration, Instant};

use tokio::runtime::Runtime;
use wireframe_testing::{
    benchmark_alloc_support::count_allocations,
    runtime_ownership_benchmarks::{PoolHarness, PoolWorkload},
};

use crate::{
    probes::expect_bench,
    scopes::{ALLOCATION_ITERATIONS, AllocationWorkloads},
};

/// Run the pool scope the way the timing pass should replay it.
///
/// One call builds a pool, measures `iterations` acquire/release rounds against
/// it, and closes it — the same workload the count beside it describes, so the
/// timing and the count are of one thing.
///
/// The returned duration covers the acquisition rounds alone. Construction and
/// closure happen outside the clock, matching the count, which brackets the same
/// rounds. `bb8` connects eagerly, so timing the whole call would report
/// `pool_size` loopback connection setups as acquisition cost — the very figure
/// [`count_pool_acquisition_window`] was written to exclude from the count.
///
/// # Errors
///
/// Returns a description of the construction, warm-up, or acquisition failure.
pub(crate) fn run_pool_acquisition(
    runtime: &Runtime,
    workload: PoolWorkload,
    iterations: u64,
) -> Result<Duration, String> {
    let harness = runtime.block_on(PoolHarness::build(workload))?;
    let started = Instant::now();
    let measured = runtime.block_on(harness.measure(iterations));
    let elapsed = started.elapsed();
    runtime.block_on(harness.close());
    measured.map(|_| elapsed)
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
pub(crate) fn count_pool_acquisition_window(
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
    // The whole-window reading is about the count, not the clock: this call
    // exists to be the unbracketed comparison, so its duration is discarded
    // rather than reported.
    let whole =
        count_allocations(|| run_pool_acquisition(runtime, workload, iterations).map(|_| ()))?;
    Ok((bracketed, whole))
}

/// Build the workload the bracket checks measure, at their own parameters.
fn bracket_workload() -> PoolWorkload { AllocationWorkloads::new().pool }

/// Prove the pool scope's bracketed window really excludes pool construction.
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
/// This half owns its own current-thread runtime and must run before any other
/// runtime exists. The counter is process-global, so a worker thread belonging
/// to a live multi-thread runtime could allocate inside the window and read as
/// pool construction. A current-thread runtime narrows that hole rather than
/// closing it: `block_on` polls only the future it is given, and `measure(0)`
/// completes without an await point, so none of the harness's spawned tasks is
/// polled. `settled_alive_tasks` is not doing this work; it bounds a gauge, and
/// a gauge that says nothing is running cannot stop a separately parked task
/// from allocating. See [`verify_measured_phase_is_bracketed`] for the other
/// half of the claim, which needs the benchmark runtime.
pub(crate) fn verify_zero_iteration_bracket_excludes_construction() {
    let workload = bracket_workload();

    let guard_runtime = expect_bench(
        tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build(),
        "zero-iteration guard runtime failed",
    );
    let (empty_bracketed, empty_whole) = expect_bench(
        pool_bracket_readings(&guard_runtime, workload, 0),
        "zero-iteration pool allocation window failed",
    );
    drop(guard_runtime);

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
}

/// Prove the bracketed window contains the acquire path, not merely nothing.
///
/// The equality in [`verify_zero_iteration_bracket_excludes_construction`] is
/// satisfied both by a correct bracket and by one that wraps nothing at all, so
/// it needs a reading at the normal iteration count to tell those apart. That
/// reading builds a real pool through the benchmark runtime, which is why this
/// half takes one as an argument instead of constructing its own.
pub(crate) fn verify_measured_phase_is_bracketed(runtime: &Runtime) {
    let workload = bracket_workload();
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
