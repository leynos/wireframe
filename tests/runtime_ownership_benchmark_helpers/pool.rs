//! V-O1 and V-O5 — the scheduler spawn counter observes `kick` wins, and pool
//! task count is stable for uncontended acquisition.
//!
//! The contract asserted here is a **bound**, not a zero. `PooledClientLease::drop`
//! calls `notify_capacity_available` unconditionally, and `try_acquire_immediately`
//! arms even a fast-path lease with a release handle, so every release reaches
//! `PoolScheduler::kick`. Measurement confirmed spawn deltas of 1 for 16-acquisition
//! bursts at pool sizes 1, 4, and 16. See the `ExecPlan`'s `Surprises &
//! discoveries` entry for the full trace.
//!
//! The lower bound of one is an *observation* on an otherwise idle runtime, not
//! an invariant of `kick`: a worker left servicing by an earlier phase can make
//! every release lose the idle-to-busy compare-exchange, and the burst then
//! spawns nothing. This test builds and warms its own pool immediately before
//! measuring, so the scheduler is idle when the burst starts and the assertion is
//! sound here; a caller reusing a pool that another phase may have just left busy
//! must assert only the upper bound.

use std::time::{Duration, Instant};

use rstest::rstest;
use wireframe_testing::runtime_ownership_benchmarks::{
    AcquisitionMode,
    POOL_SIZES,
    PoolWorkload,
    measure_pool_acquisition,
};

use super::harness::{ITERATIONS, block_on, block_on_current_thread, contended_burst_with_retry};

#[rstest]
#[case::single(1)]
#[case::small(4)]
#[case::large(16)]
fn uncontended_acquisition_spawns_a_bounded_number_of_scheduler_tasks(#[case] pool_size: usize) {
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

    // The witness that makes the uncontended bounds meaningful. Two claims are
    // needed, and neither implies the other.
    //
    // Capacity waits prove the burst actually contended: the pool was emptied
    // and held empty while every contender asked for capacity, so each contender
    // had to park. This is the property a start barrier alone cannot establish,
    // because a task that wins a permit can complete several immediately-ready
    // acquisitions before another is polled.
    assert!(
        measurement.capacity_waits >= u64::try_from(contenders).unwrap_or(u64::MAX),
        "a burst that empties a {pool_size}-slot pool and holds it empty must park every one of \
         {contenders} contenders: {} waits recorded",
        measurement.capacity_waits
    );
    assert!(
        measurement.contended(),
        "the contended measurement must classify itself as contended"
    );
    // Spawns are the task-churn evidence, kept beside the contention witness
    // rather than standing in for it: every release notifies the scheduler
    // whether or not anyone is waiting, so a moving spawn count says the
    // scheduler was entered, not that a caller had to wait.
    assert!(
        measurement.scheduler_spawns > 0,
        "handing capacity back to parked waiters must enter the scheduler slow path at least once"
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
/// the named baseline machine, before #535: the first failure lands at burst
/// 913 of 20,000 at 16 contenders, and at roughly 1 round in 400 for the
/// lighter 8-contender workload below. Re-tested after #535, the same workload
/// fails at roughly 1 round in 73 on the FIFO policy and 1 in 632 on
/// round-robin.
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
