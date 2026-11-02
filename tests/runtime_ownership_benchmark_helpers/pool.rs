//! V-O1 and V-O5 — acquisition never spawns a scheduler task, and the live task
//! count is stable across uncontended acquisition.
//!
//! Each pool starts exactly one scheduler task when it is constructed, and that
//! task serves every later acquisition from its mailbox. Construction happens
//! before the measurement bracket opens, so a measured phase must report a
//! spawn delta of **zero** however many leases it grants. The task is long
//! lived, so it is present in both live-task readings and cancels out of the
//! delta rather than appearing as churn.
//!
//! `capacity_waits` remains the direct witness that a contended burst really
//! queued: a spawn count can no longer stand in for that, because it never
//! moves during a measured phase.

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
fn uncontended_acquisition_spawns_no_scheduler_tasks(#[case] pool_size: usize) {
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
    // The pool's scheduler task was started by construction, which the bracket
    // excludes. Acquiring and releasing leases is mailbox traffic to a task
    // that already exists, so it can never spawn another.
    assert_eq!(
        measurement.scheduler_spawns, 0,
        "acquisition must not start a scheduler task; the pool owns exactly one"
    );
    // The live-task delta cancels the persistent scheduler task out: it is
    // alive before the first measured phase and still alive after the last.
    // What remains is any unrelated runtime task, which this bound keeps small
    // rather than pinning to a scheduling accident. A per-acquisition spawn
    // would move the delta by one per lease and fail this immediately.
    assert!(
        measurement.alive_task_delta() <= 2,
        "uncontended acquisition over {pool_size} slots must not grow the live task count: saw {}",
        measurement.alive_task_delta()
    );
}

#[rstest]
#[case::fifo(AcquisitionMode::ContendedFifo)]
#[case::round_robin(AcquisitionMode::ContendedRoundRobin)]
fn contended_acquisition_spawns_no_additional_scheduler_tasks(#[case] mode: AcquisitionMode) {
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
    // Contention is served by the pool's existing task, so the spawn count must
    // stay flat even while callers queue. This is the counterpart to the
    // capacity-wait witness above: waiters really parked, and parking them cost
    // no new task.
    assert_eq!(
        measurement.scheduler_spawns, 0,
        "serving parked waiters must reuse the pool's scheduler task, not spawn another"
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

/// Contended acquisition must never refuse a caller with a spurious
/// `disconnected` error.
///
/// The pre-#647 scheduler reported `disconnected` whenever it dequeued a waiter
/// whose session had stopped listening. A contended round reached that state
/// naturally: a caller that abandoned an acquisition left its sender queued, so
/// a healthy lease could be discarded while the pool was still serving traffic.
/// The persistent scheduler prunes cancelled waiters before selecting one and
/// refuses callers with typed pool-lifecycle errors instead, so no round may
/// fail for that reason.
///
/// The condition is reproduced on the per-round pools the benchmark builds,
/// which is where the earlier measurement observed it: roughly one round in 73
/// on the FIFO policy and one in 632 on round-robin, so 400 rounds makes a
/// regression very likely to show.
#[rstest]
#[case::fifo(AcquisitionMode::ContendedFifo)]
#[case::round_robin(AcquisitionMode::ContendedRoundRobin)]
fn contended_acquisition_is_never_refused_with_a_spurious_disconnect(
    #[case] mode: AcquisitionMode,
) {
    /// Rounds attempted, matched to the recorded pre-fix failure rate.
    const ROUNDS: usize = 400;

    /// Wall-clock budget for the whole loop.
    ///
    /// A healthy run completes 400 rounds in about two seconds. The budget
    /// exists for the pathological case: a stalled round costs the full
    /// `CONTENDED_BURST_TIMEOUT` (30 s), so without a bound a machine that
    /// stalled repeatedly would hold this test — and the gate — for an
    /// unbounded time. Reaching the budget stops the loop and reports the
    /// failures seen so far, which fail the assertion below.
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

    assert!(attempted > 0, "no round completed within {TIME_BUDGET:?}");
    assert!(
        failures.is_empty(),
        "contended acquisition must serve every round: {} of {attempted} failed: {failures:?}",
        failures.len()
    );
}
