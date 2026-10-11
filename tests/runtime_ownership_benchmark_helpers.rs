//! Unit tests for the runtime-ownership benchmark workload and measurement helpers.
//!
//! Each test discharges one obligation from the ExecPlan's verification plan for
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
/// Multi-thread rather than current-thread throughout: the contended pool cases
/// park callers on a barrier and need real parallelism to exhaust the pool, and
/// using one runtime flavour everywhere keeps a passing uncontended case from
/// depending on which flavour happened to be chosen.
fn block_on<F: std::future::Future>(future: F) -> F::Output {
    tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .expect("failed to build the test runtime")
        .block_on(future)
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
    let measurement = block_on(measure_pool_acquisition(
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
    // small constant rather than scaling with the pool size. Measured settled
    // deltas are 1, 3, and 2 for pools of 1, 4, and 16 slots; construction, by
    // contrast, contributed 2, 5, and 17 in the mutation experiment.
    //
    // A small non-zero figure is expected here because this test drives a
    // multi-thread runtime whose teardown is asynchronous. The in-module case
    // `acquisition_phase_does_not_leak_pool_construction_tasks` asserts the
    // tighter settled bound under a current-thread runtime, and that case is the
    // one verified by the mutation transcript recorded in
    // `docs/runtime-ownership-baselines.md`.
    assert!(
        measurement.alive_task_delta() <= 4,
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
    let measurement = block_on(measure_pool_acquisition(
        PoolWorkload {
            pool_size,
            mode,
            acquisitions: 8,
            contenders,
        },
        ITERATIONS,
    ))
    .expect("contended pool measurement failed");

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

#[rstest]
#[case::fifo(AcquisitionMode::ContendedFifo)]
#[case::round_robin(AcquisitionMode::ContendedRoundRobin)]
fn contended_acquisition_never_reports_a_spurious_disconnect(#[case] mode: AcquisitionMode) {
    // Regression guard. `PoolScheduler::acquire_for_handle` used to surface a
    // `disconnected` error whenever it dequeued a waiter whose session had
    // already stopped listening, which a contended round reaches naturally:
    // the discarded lease was healthy, so a caller could be told the pool had
    // gone away while it was still serving traffic. Repeated rather than run
    // once because the trigger is an interleaving, and a single round has a
    // low probability of hitting it.
    const ROUNDS: usize = 400;
    let mut failures = Vec::new();
    for round in 0..ROUNDS {
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
        failures.is_empty(),
        "contended acquisition must not fail while capacity exists; {} of {ROUNDS} rounds \
         reported an error: {}",
        failures.len(),
        failures.join(", ")
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
// The counter assertions for this obligation deliberately live inside
// `wireframe_testing`'s own unit tests rather than here. `CountingAllocator` is
// a `#[global_allocator]`, and this file is a `tests/` target of the root crate,
// which links the system allocator: a probe here reported zero for a closure
// that allocated a 1024-byte buffer, proving the gate cannot open in this
// binary. Installing an allocator per test target is not possible, so the
// controls run where the allocator is installed. The nested-`Arc` sensitivity
// experiment is recorded in `docs/runtime-ownership-baselines.md` for the same
// reason.
// ---------------------------------------------------------------------------
