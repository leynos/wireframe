//! Unit tests for the runtime-ownership benchmark workload and measurement helpers.
//!
//! Each test discharges one obligation from the `ExecPlan`'s verification plan for
//! issue #639: V-O1 through V-O5 here, with V-O6's allocation controls living in
//! `benches/runtime_ownership_baselines_alloc.rs` because they need a
//! `#[global_allocator]` that a `tests/` target cannot install. Those
//! obligations are counting contracts over finite partitions, so parameterized
//! `rstest` cases carry them rather than property tests or a formal method; a
//! generator over these integer partitions would restate the same grid, and a
//! proof would restate the assertions.
//!
//! These checks drive real runtimes, allocate, and open loopback connections, so
//! they are unsuitable for loom.

#![cfg(not(loom))]

use std::time::{Duration, Instant};

use rstest::rstest;
use wireframe_testing::runtime_ownership_benchmarks::{
    AcquisitionMode,
    ActorWorkload,
    CancellationMode,
    DEFAULT_ACTOR_QUEUE_CAPACITY,
    DEFAULT_CONNECTIONS,
    DEFAULT_FRAME_COUNT,
    DEFAULT_MIDDLEWARE_LAYERS,
    DEFAULT_ROUTES,
    POOL_SIZES,
    PoolMeasurement,
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

/// Workload iterations used by the assertion tests.
///
/// One suffices for every count asserted here: each helper reports the *last*
/// iteration's counters, so repeating an iteration would only re-assert the same
/// expected value. Iteration-count behaviour has its own cases.
const ITERATIONS: u64 = 1;

/// Run a future to completion on a fresh multi-thread runtime.
///
/// Multi-thread for the workload assertions because the contended pool cases
/// park callers on a barrier and need real parallelism to exhaust the pool. The
/// one case that reads the live-task gauge instead uses
/// [`block_on_current_thread`], for the reason documented there.
///
/// Returns `Result` rather than panicking on a runtime-construction failure:
/// this helper lives outside any `#[test]`, so the workspace's test-only
/// `expect_used` allowance does not cover it. The error type is the workloads'
/// own `String` so the result flattens into the one `Result` each caller
/// already matches on, which keeps the failure surfaced without adding a panic
/// path to the helper.
fn block_on<F, T>(future: F) -> Result<T, String>
where
    F: std::future::Future<Output = Result<T, String>>,
{
    tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .map_err(|error| format!("failed to build the test runtime: {error}"))?
        .block_on(future)
}

/// Run a future to completion on a fresh current-thread runtime.
///
/// Only the live-task-gauge assertion uses this, and the choice is forced by
/// measurement rather than taste. `RuntimeMetrics::num_alive_tasks` counts
/// *every* task on the runtime, including ones this workload never created, so
/// on a multi-thread runtime it is a whole-runtime gauge, not a gauge on the
/// pool. Sampling it there produced deltas from −12 to +10 with a long tail
/// above +2 in roughly a third of runs, at every pool size, on a workload whose
/// own task churn is a fixed small constant.
///
/// On a current-thread runtime the same workload is deterministic: 180 samples
/// across pool sizes 1, 4, and 16 recorded only −1 and 0, with no outlier. That
/// is the regime in which the reading answers the question the assertion asks,
/// so the assertion is made here and the multi-thread flavour is left to the
/// cases that need its parallelism.
///
/// Uncontended acquisition does not need parallelism to be exercised: it is a
/// straight-line acquire/release loop. Only the contended cases park on a
/// barrier, and they assert counts rather than the gauge.
///
/// Error handling matches [`block_on`].
fn block_on_current_thread<F, T>(future: F) -> Result<T, String>
where
    F: std::future::Future<Output = Result<T, String>>,
{
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .map_err(|error| format!("failed to build the test runtime: {error}"))?
        .block_on(future)
}

/// Attempts allowed before a contended burst is reported as a failure.
///
/// The contended acquire path intermittently fails a burst with a spurious
/// `disconnected` (roughly one burst in 913 at 16 contenders on the named
/// baseline machine) or stalls until
/// [`CONTENDED_BURST_TIMEOUT`] converts the stall into an error. Both are
/// pre-existing defects recorded against issue #647. The case below asserts
/// what a contended burst *can* produce, so it retries a bounded number of
/// times instead of failing the suite on an interleaving unrelated to the
/// change under review.
const CONTENDED_BURST_ATTEMPTS: usize = 8;

/// Run one contended burst, retrying the acquire path's known spurious failures.
///
/// Returns the first successful observation, or the last error if every attempt
/// failed. Consumes a future factory rather than a future because each attempt
/// needs a fresh measurement; the caller supplies it so this helper carries no
/// policy about what is being measured.
///
/// [`CONTENDED_BURST_TIMEOUT`]: wireframe_testing::runtime_ownership_benchmarks::CONTENDED_BURST_TIMEOUT
fn contended_burst_with_retry<F, Fut>(mut attempt: F) -> Result<PoolMeasurement, String>
where
    F: FnMut() -> Fut,
    Fut: std::future::Future<Output = Result<PoolMeasurement, String>>,
{
    let mut last_error = None;
    for _ in 0..CONTENDED_BURST_ATTEMPTS {
        match block_on(attempt()) {
            Ok(measurement) => return Ok(measurement),
            Err(error) => last_error = Some(error),
        }
    }

    Err(format!(
        "contended burst failed in all {CONTENDED_BURST_ATTEMPTS} attempts; last: {}",
        last_error.as_deref().unwrap_or("no error recorded")
    ))
}

// ---------------------------------------------------------------------------
// V-O2 — factory and middleware counts are exact, not inferred.
// ---------------------------------------------------------------------------

#[rstest]
#[case::one_route_one_layer(1, 1, 1)]
#[case::two_routes_two_layers(2, 2, 4)]
#[case::four_routes_three_layers(4, 3, 12)]
fn prepared_root_transforms_once_regardless_of_connection_count(
    #[case] routes: usize,
    #[case] middleware_layers: usize,
    #[case] connections: usize,
) {
    let measurement = block_on(measure_startup(
        StartupWorkload {
            path: StartupPath::PreparedRoot,
            routes,
            middleware_layers,
            connections,
        },
        ITERATIONS,
    ))
    .expect("prepared-root startup measurement failed");

    assert_eq!(
        measurement.factory_calls, 1,
        "a shared prepared root is built once for all {connections} connections"
    );
    assert_eq!(
        measurement.transforms,
        routes * middleware_layers,
        "middleware is applied once per route per layer, not once per connection"
    );
}

#[rstest]
#[case::one_route_one_layer(1, 1, 1)]
#[case::two_routes_two_layers(2, 2, 4)]
#[case::four_routes_three_layers(4, 3, 12)]
fn per_connection_factory_transforms_for_every_connection(
    #[case] routes: usize,
    #[case] middleware_layers: usize,
    #[case] connections: usize,
) {
    let measurement = block_on(measure_startup(
        StartupWorkload {
            path: StartupPath::PerConnectionFactory,
            routes,
            middleware_layers,
            connections,
        },
        ITERATIONS,
    ))
    .expect("per-connection startup measurement failed");

    assert_eq!(
        measurement.factory_calls, connections,
        "the per-connection path builds a fresh application for each connection"
    );
    assert_eq!(
        measurement.transforms,
        connections * routes * middleware_layers,
        "every connection re-applies the whole middleware chain"
    );
}

#[test]
fn startup_counts_appear_in_the_benchmark_label() {
    let workload = StartupWorkload {
        path: StartupPath::PreparedRoot,
        routes: 2,
        middleware_layers: 2,
        connections: 4,
    };
    let label = workload.label();
    assert!(
        label.contains("factory_1") && label.contains("transforms_4"),
        "the label must carry the exact counts a baseline table quotes: {label}"
    );
}

// ---------------------------------------------------------------------------
// V-O3 — the actor drains exactly the queued frames, and the cancellation modes
// are distinguishable.
// ---------------------------------------------------------------------------

#[rstest]
#[case::single(1)]
#[case::awkward_odd(17)]
#[case::bulk(256)]
fn actor_drains_every_queued_frame_with_cancellation_disabled(#[case] frames: usize) {
    let measurement = block_on(measure_actor_drain(
        ActorWorkload {
            frames,
            queue_capacity: DEFAULT_ACTOR_QUEUE_CAPACITY,
            cancellation: CancellationMode::Disabled,
        },
        ITERATIONS,
    ))
    .expect("actor drain measurement failed");

    assert_eq!(
        measurement.frames_emitted, frames,
        "the loop must emit every preloaded frame, across both priority lanes"
    );
    assert!(
        !measurement.cancelled_during_drain,
        "the disabled regime must not record an observed cancellation"
    );
}

#[test]
fn actor_emits_nothing_when_cancelled_before_it_runs() {
    let measurement = block_on(measure_actor_drain(
        ActorWorkload {
            frames: 256,
            queue_capacity: DEFAULT_ACTOR_QUEUE_CAPACITY,
            cancellation: CancellationMode::CancelledBeforeRun,
        },
        ITERATIONS,
    ))
    .expect("cancelled actor drain measurement failed");

    assert_eq!(
        measurement.frames_emitted, 0,
        "a token cancelled before the run must abort the loop before it drains"
    );
}

#[test]
fn actor_still_emits_every_frame_when_cancelled_mid_drain() {
    let frames = 256;
    let measurement = block_on(measure_actor_drain(
        ActorWorkload {
            frames,
            queue_capacity: DEFAULT_ACTOR_QUEUE_CAPACITY,
            cancellation: CancellationMode::CancelledDuringDrain,
        },
        ITERATIONS,
    ))
    .expect("mid-drain cancelled actor measurement failed");

    assert!(
        measurement.cancelled_during_drain,
        "the output hook must have observed the cancellation, otherwise this case is \
         indistinguishable from the disabled regime"
    );
    assert_eq!(
        measurement.frames_emitted, frames,
        "observing shutdown mid-drain must not abandon frames already queued"
    );
}

#[test]
fn default_actor_workload_fits_its_queues() {
    let workload = ActorWorkload {
        frames: DEFAULT_FRAME_COUNT,
        queue_capacity: DEFAULT_ACTOR_QUEUE_CAPACITY,
        cancellation: CancellationMode::Disabled,
    };
    // The helper rejects an over-capacity workload, so a successful measurement
    // is itself the assertion that the default fits without back-pressure.
    block_on(measure_actor_drain(workload, ITERATIONS))
        .expect("the default actor workload must fit its queues");
}

// ---------------------------------------------------------------------------
// V-O4 — the push scenarios produce their documented outcomes.
// ---------------------------------------------------------------------------

#[rstest]
#[case::high(PushScenario::UncontendedHigh)]
#[case::low(PushScenario::UncontendedLow)]
fn uncontended_sends_are_all_accepted_and_drained(#[case] scenario: PushScenario) {
    let sends = 32;
    let measurement = block_on(measure_push(
        PushWorkload {
            scenario,
            queue_capacity: 64,
            sends,
        },
        ITERATIONS,
    ))
    .expect("uncontended push measurement failed");

    assert_eq!(
        measurement.accepted, sends,
        "every awaited send must be accepted"
    );
    assert_eq!(
        measurement.drained, sends,
        "a draining consumer must receive every frame, so none were lost"
    );
    assert_eq!(
        measurement.dead_lettered, 0,
        "the uncontended path must not route anything to the dead-letter queue"
    );
    assert!(
        !measurement.reported_queue_full,
        "an awaited send parks rather than reporting a full queue"
    );
}

#[rstest]
#[case::one(1)]
#[case::small(4)]
#[case::default(64)]
fn overflow_return_error_rejects_without_dead_lettering(#[case] queue_capacity: usize) {
    let measurement = block_on(measure_push(
        PushWorkload {
            scenario: PushScenario::OverflowReturnError,
            queue_capacity,
            sends: 8,
        },
        ITERATIONS,
    ))
    .expect("overflow return-error measurement failed");

    assert!(
        measurement.reported_queue_full,
        "`ReturnErrorIfFull` must surface `QueueFull` on a full queue"
    );
    assert_eq!(
        measurement.accepted, 0,
        "no send under `ReturnErrorIfFull` may report success against a full queue"
    );
    assert_eq!(
        measurement.dead_lettered, 0,
        "`ReturnErrorIfFull` returns the frame to the caller rather than the DLQ"
    );
}

#[rstest]
#[case::drop(PushScenario::OverflowDrop)]
#[case::warn_and_drop(PushScenario::OverflowWarnAndDrop)]
fn overflow_drop_routes_every_rejected_frame_to_the_dead_letter_queue(
    #[case] scenario: PushScenario,
) {
    let sends = 8;
    let measurement = block_on(measure_push(
        PushWorkload {
            scenario,
            queue_capacity: 4,
            sends,
        },
        ITERATIONS,
    ))
    .expect("overflow drop measurement failed");

    // A drop policy reports `Ok(())` for a frame it did *not* queue, so
    // `accepted` counts completed send paths rather than queued frames. The DLQ
    // count is what distinguishes the two, which is why both are asserted.
    assert_eq!(
        measurement.accepted, sends,
        "a drop policy reports that the send path completed"
    );
    assert_eq!(
        measurement.dead_lettered, sends,
        "every dropped frame must be observable on the dead-letter queue"
    );
    assert!(
        !measurement.reported_queue_full,
        "the drop policies swallow the full-queue condition rather than reporting it"
    );
    assert_eq!(
        measurement.drained, 0,
        "nothing was drained, so a non-zero count would mean a frame was lost"
    );
}

// ---------------------------------------------------------------------------
// V-O1 and V-O5 — the scheduler spawn counter observes `kick` wins, and pool
// task count is stable for uncontended acquisition.
//
// The contract asserted here is a **bound**, not a zero. `PooledClientLease::drop`
// calls `notify_capacity_available` unconditionally, and `try_acquire_immediately`
// arms even a fast-path lease with a release handle, so every release reaches
// `PoolScheduler::kick`. Measurement confirmed spawn deltas of 1 for 16-acquisition
// bursts at pool sizes 1, 4, and 16. See the ExecPlan's `Surprises & discoveries`
// entry for the full trace.
// ---------------------------------------------------------------------------

#[rstest]
#[case::single(1)]
#[case::small(4)]
#[case::large(16)]
fn uncontended_acquisition_does_not_spawn_pool_scheduler_tasks(#[case] pool_size: usize) {
    let acquisitions = 16;
    // Current-thread runtime *only* for this case: it is the one case that reads
    // the live-task gauge, and that gauge is only meaningful when the runtime is
    // not also running unrelated work. See `block_on_current_thread`.
    let measurement = block_on_current_thread(measure_pool_acquisition(
        PoolWorkload {
            pool_size,
            mode: AcquisitionMode::Uncontended,
            acquisitions,
            contenders: 16,
        },
        ITERATIONS,
    ))
    .expect("uncontended pool measurement failed");

    assert_eq!(
        measurement.acquisitions,
        u64::try_from(acquisitions).unwrap_or(u64::MAX),
        "every lease the caller asked for must be granted"
    );
    // The slow path is entered at least once per burst, but only on an
    // idle-to-busy transition, so the count is bounded rather than free.
    assert!(
        measurement.scheduler_spawns >= 1,
        "a release always notifies the scheduler, so a burst must spawn at least one worker"
    );
    let transitions = u64::try_from(acquisitions.saturating_mul(2)).unwrap_or(u64::MAX);
    assert!(
        measurement.scheduler_spawns <= transitions,
        "spawns ({}) must not exceed the {} acquire/release transitions",
        measurement.scheduler_spawns,
        transitions
    );
    // The bracket excludes pool construction, so the settled delta must be a
    // small constant rather than scaling with the pool size. Measured over 180
    // current-thread runs at pool sizes 1, 4, and 16, the delta was only ever
    // −1 or 0 — a *negative* delta, because a burst can leave a pre-existing
    // worker idle-exiting. Construction, by contrast, contributed 2, 5, and 17
    // when the bracket was moved to include it, scaling with `pool_size`.
    //
    // The bound is a non-scaling ceiling rather than a tight fit to those two
    // observed values: pinning it to `<= 0` would encode a scheduling accident,
    // and the property under test is "does not grow with the pool", not "happens
    // to settle at or below zero". A ceiling of 2 would still fail on all three
    // mutated cases above, so it discriminates the failure the bracket exists to
    // catch while leaving headroom for a scheduler that has not quiesced yet.
    //
    // This is the same property the in-module
    // `acquisition_phase_does_not_leak_pool_construction_tasks` asserts, from
    // outside the support crate; that case is the one verified by the mutation
    // transcript recorded in `docs/runtime-ownership-baselines.md`.
    assert!(
        measurement.alive_task_delta() <= 2,
        "a closed {pool_size}-slot pool must settle to a small constant task delta, not one that \
         scales with the pool: saw {}",
        measurement.alive_task_delta()
    );
}

#[rstest]
#[case::fifo(AcquisitionMode::ContendedFifo)]
#[case::round_robin(AcquisitionMode::ContendedRoundRobin)]
fn contended_acquisition_does_spawn_pool_scheduler_tasks(#[case] mode: AcquisitionMode) {
    let pool_size = 4;
    let contenders = 16;
    let measurement = contended_burst_with_retry(|| {
        measure_pool_acquisition(
            PoolWorkload {
                pool_size,
                mode,
                acquisitions: 8,
                contenders,
            },
            ITERATIONS,
        )
    })
    .expect("contended pool measurement failed in every retry attempt");

    // This is the witness that makes the uncontended bound meaningful: the
    // counter demonstrably moves under demand, so its behaviour above is a
    // measured property rather than a counter wired to nothing.
    assert!(
        measurement.scheduler_spawns > 0,
        "exhausting a {pool_size}-slot pool with {contenders} contenders must enter the scheduler \
         slow path at least once"
    );
}

#[test]
fn contended_workload_needs_more_contenders_than_slots() {
    // A contended workload that cannot exhaust the pool would report an
    // uncontended finding under a contended label, so it is rejected.
    let error = block_on(measure_pool_acquisition(
        PoolWorkload {
            pool_size: 16,
            mode: AcquisitionMode::ContendedFifo,
            acquisitions: 8,
            contenders: 4,
        },
        ITERATIONS,
    ))
    .expect_err("contenders that cannot exhaust the pool must be rejected");

    assert!(
        error.contains("needs more contenders than pool slots"),
        "unexpected rejection reason: {error}"
    );
}

#[test]
fn pool_sizes_cover_the_issue_requirement() {
    assert_eq!(
        POOL_SIZES,
        [1, 4, 16],
        "the issue names pool sizes 1, 4, and 16 explicitly"
    );
}

/// Contended acquisition is **known** to surface spurious `disconnected`
/// errors, and this test pins that known-unsound behaviour rather than
/// pretending it is fixed.
///
/// `PoolScheduler::acquire_for_handle` reports `disconnected` whenever it
/// dequeues a waiter whose session has already stopped listening. A contended
/// round reaches that state naturally: a caller that abandons an acquisition
/// leaves its sender queued, and `notify_shutdown` abandons receivers outright.
/// The lease being discarded is healthy, so a caller can be told the pool has
/// gone away while the pool is still serving traffic.
///
/// The condition is reproduced here on a *long-lived* pool (no rebuild, no task
/// leak), which is what proves it is an acquire-path defect rather than an
/// artefact of the benchmark's per-iteration pool construction. Measurement on
/// the named baseline machine: the first failure lands at burst 913 of 20,000
/// at 16 contenders, and at roughly 1 round in 400 for the lighter
/// 8-contender workload below.
///
/// This is deliberately an *allowed failure* rather than an assertion of
/// correctness, and it is deliberately **not** a pin on the defect: the
/// assertion tolerates every outcome from "no round failed" to "all but one
/// round failed", so fixing the acquire path will not make it fail. What it
/// rules out is the strictly worse state in which *no* round succeeds, which
/// would be a different and more serious defect than the interleaving
/// recorded here. Filed under ADR 013
/// (`docs/adr-013-client-pool-scheduler-and-slot-ownership.md`) and issue #647.
#[rstest]
#[case::fifo(AcquisitionMode::ContendedFifo)]
#[case::round_robin(AcquisitionMode::ContendedRoundRobin)]
fn contended_acquisition_may_report_a_known_spurious_disconnect(#[case] mode: AcquisitionMode) {
    /// Rounds attempted, matched to the documented failure rate: at roughly one
    /// failing round in 400, this many rounds is what makes an observed failure
    /// likely rather than lucky.
    const ROUNDS: usize = 400;

    /// Floor on how many rounds must be attempted before an observation counts.
    ///
    /// Only reachable when the acquire path stalls far more often than the
    /// recorded rate, since each stall is capped at `CONTENDED_BURST_TIMEOUT`
    /// and the budget below allows several. Failing here means the path is
    /// behaving worse than the recorded defect, which is worth surfacing rather
    /// than reporting as a clean observation.
    const MIN_ATTEMPTS: usize = 32;

    /// Wall-clock budget for the whole loop.
    ///
    /// A healthy run completes 400 rounds in about two seconds. The budget
    /// exists for the pathological case: a stalled round costs the full
    /// `CONTENDED_BURST_TIMEOUT` (30 s), so without a bound a machine that
    /// stalled on many rounds would hold this test — and the gate — for an
    /// unbounded time. Reaching the budget stops the loop and reports how far
    /// it got instead of discarding the observation made so far.
    const TIME_BUDGET: Duration = Duration::from_mins(3);

    let started = Instant::now();
    let mut attempted = 0usize;
    let mut failures = Vec::new();
    for round in 0..ROUNDS {
        if started.elapsed() > TIME_BUDGET {
            break;
        }
        attempted += 1;
        if let Err(error) = block_on(measure_pool_acquisition(
            PoolWorkload {
                pool_size: 4,
                mode,
                acquisitions: 16,
                contenders: 8,
            },
            ITERATIONS,
        )) {
            failures.push(format!("round {round}: {error}"));
        }
    }

    assert!(
        attempted >= MIN_ATTEMPTS,
        "only {attempted} of {ROUNDS} rounds completed within {TIME_BUDGET:?} ({failures} \
         failed), so the acquire path is stalling far more often than the recorded defect; the \
         observation is inconclusive rather than clean",
        failures = failures.len()
    );

    // The tolerance is "some rounds may fail", not "all rounds may fail": a
    // pool that failed *every* round would be broken in a different and much
    // more serious way than the known interleaving defect.
    assert!(
        failures.len() < attempted,
        "contended acquisition failed in every one of the {attempted} rounds attempted, which is \
         a broader failure than the known spurious-disconnect interleaving"
    );
}

// ---------------------------------------------------------------------------
// Workload defaults, and the label contract the baseline table depends on.
// ---------------------------------------------------------------------------

#[test]
fn default_workloads_are_usable_without_configuration() {
    let startup = StartupWorkload::default();
    assert_eq!(startup.routes, DEFAULT_ROUTES);
    assert_eq!(startup.middleware_layers, DEFAULT_MIDDLEWARE_LAYERS);
    assert_eq!(startup.connections, DEFAULT_CONNECTIONS);

    let pool = PoolWorkload::default();
    assert!(
        pool.pool_size > 0,
        "a default pool must have at least one slot"
    );

    assert_eq!(ActorWorkload::default().frames, DEFAULT_FRAME_COUNT);
}

#[rstest]
#[case::actor(ActorWorkload::default().label())]
#[case::push(PushWorkload::default().label())]
#[case::startup(StartupWorkload::default().label())]
#[case::pool(PoolWorkload::default().label())]
fn labels_are_identifier_safe(#[case] label: String) {
    assert!(!label.is_empty(), "labels must not be empty");
    assert!(
        !label.contains(char::is_whitespace),
        "labels become Criterion identifiers, so they must not contain whitespace: {label}"
    );
}

// ---------------------------------------------------------------------------
// V-O6 — the allocation harness can detect a change in allocation count.
//
// The counter assertions for this obligation live in `verify_allocation_harness`
// in `benches/runtime_ownership_baselines_alloc.rs`, not here and not in
// `wireframe_testing`'s unit tests. `CountingAllocator` is a
// `#[global_allocator]`, and this file is a `tests/` target of the root crate,
// which links the system allocator: a probe here reported zero for a closure
// that allocated a 1024-byte buffer, proving the gate cannot open in this
// binary. Installing an allocator per test target is not possible, so the
// controls run where the allocator is installed. `wireframe_testing`'s own unit
// tests can only check that the counting window opens and closes, since the gate
// is observable without the allocator; the counts themselves are asserted in the
// bench binary. The nested-`Arc` sensitivity experiment is recorded in
// `docs/runtime-ownership-baselines.md` for the same reason.
// ---------------------------------------------------------------------------
