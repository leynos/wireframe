//! Unit coverage for pool workload labels, rejection rules, and the counters.

use super::{AcquisitionMode, PoolWorkload, measure_pool_acquisition};

#[test]
fn uncontended_mode_runs_exactly_one_contender() {
    let workload = PoolWorkload {
        pool_size: 4,
        mode: AcquisitionMode::Uncontended,
        acquisitions: 8,
        contenders: 64,
    };
    assert!(
        workload.label().ends_with("contenders_1"),
        "uncontended acquisition must not fan out: {}",
        workload.label()
    );
}

#[tokio::test]
async fn contended_mode_needs_more_contenders_than_slots() {
    let error = measure_pool_acquisition(
        PoolWorkload {
            pool_size: 16,
            mode: AcquisitionMode::ContendedFifo,
            acquisitions: 8,
            contenders: 4,
        },
        1,
    )
    .await
    .expect_err("contenders that cannot exhaust the pool must be rejected");

    assert!(
        error.contains("needs more contenders than pool slots"),
        "unexpected: {error}"
    );
}

#[tokio::test]
async fn zero_iterations_measures_nothing() {
    let measurement = measure_pool_acquisition(PoolWorkload::default(), 0)
        .await
        .expect("zero iterations must succeed");

    assert_eq!(measurement.acquisitions, 0);
    assert_eq!(measurement.scheduler_spawns, 0);
    assert_eq!(measurement.alive_task_delta(), 0);
}

/// Every lease release reaches `kick`, so a burst always spawns something.
///
/// This is the measurement that falsified the "uncontended spawns nothing"
/// assumption. It is recorded as a test rather than a comment so the corrected
/// contract is pinned: if a later refactor removes the per-release notify, this
/// fails and the baseline table must be revisited.
#[rstest::rstest]
#[case(1, 1)]
#[case(1, 16)]
#[case(4, 16)]
#[case(16, 16)]
#[tokio::test]
async fn uncontended_acquisition_spawns_at_least_one_scheduler_task(
    #[case] pool_size: usize,
    #[case] acquisitions: usize,
) {
    let measurement = measure_pool_acquisition(
        PoolWorkload {
            pool_size,
            mode: AcquisitionMode::Uncontended,
            acquisitions,
            contenders: 1,
        },
        1,
    )
    .await
    .expect("uncontended measurement failed");

    assert_eq!(
        measurement.acquisitions,
        u64::try_from(acquisitions).unwrap_or(u64::MAX),
        "every requested lease must be granted"
    );
    assert!(
        measurement.scheduler_spawns >= 1,
        "a release always notifies the scheduler, so a {acquisitions}-acquisition burst on a \
         {pool_size}-slot pool must spawn at least one worker"
    );
}

/// The spawn count is bounded by the number of acquire/release transitions.
///
/// `kick` spawns only on an idle-to-busy compare-exchange, so a burst cannot
/// spawn more workers than it performs transitions. This is what makes the
/// measurement a *bound* rather than an arbitrary number.
#[rstest::rstest]
#[case(1, 4)]
#[case(4, 16)]
#[case(16, 64)]
#[tokio::test]
async fn uncontended_spawns_are_bounded_by_transitions(
    #[case] pool_size: usize,
    #[case] acquisitions: usize,
) {
    let measurement = measure_pool_acquisition(
        PoolWorkload {
            pool_size,
            mode: AcquisitionMode::Uncontended,
            acquisitions,
            contenders: 1,
        },
        1,
    )
    .await
    .expect("uncontended measurement failed");

    let transitions = u64::try_from(acquisitions.saturating_mul(2)).unwrap_or(u64::MAX);
    assert!(
        measurement.scheduler_spawns <= transitions,
        "spawns ({}) must not exceed the {} acquire/release transitions",
        measurement.scheduler_spawns,
        transitions
    );
}

/// A contended burst drives the slow path at least as hard as uncontended.
#[rstest::rstest]
#[case(AcquisitionMode::ContendedFifo)]
#[case(AcquisitionMode::ContendedRoundRobin)]
#[tokio::test]
async fn contended_acquisition_spawns_scheduler_tasks(#[case] mode: AcquisitionMode) {
    let measurement = measure_pool_acquisition(
        PoolWorkload {
            pool_size: 4,
            mode,
            acquisitions: 8,
            contenders: 16,
        },
        1,
    )
    .await
    .expect("contended measurement failed");

    assert!(
        measurement.scheduler_spawns >= 1,
        "exhausting a pool must enter the scheduler slow path at least once"
    );
    assert!(
        measurement.acquisitions >= 1,
        "contended contenders must still be granted leases"
    );
}

/// The bracketed region excludes pool construction.
///
/// `bb8` connects eagerly, so a size-N pool leaves N connection tasks alive.
/// If the gauge were sampled across construction it would report roughly N
/// here; this asserts the settled behaviour the bracket is meant to give.
#[rstest::rstest]
#[case(1)]
#[case(4)]
#[case(16)]
#[tokio::test]
async fn acquisition_phase_does_not_leak_pool_construction_tasks(#[case] pool_size: usize) {
    let measurement = measure_pool_acquisition(
        PoolWorkload {
            pool_size,
            mode: AcquisitionMode::Uncontended,
            acquisitions: 16,
            contenders: 1,
        },
        1,
    )
    .await
    .expect("uncontended measurement failed");

    // The pool is closed before the measurement returns, so the settled
    // delta cannot still show the eagerly-built connections. `<= 0` is
    // affordable here because `#[tokio::test]` defaults to a current-thread
    // runtime, where this gauge is deterministic; the same assertion made on
    // a multi-thread runtime would be reading the whole runtime's scheduler.
    assert!(
        measurement.alive_task_delta() <= 0,
        "a closed pool must not leave construction tasks behind: delta {} on a {pool_size}-slot \
         pool (before {}, after {})",
        measurement.alive_task_delta(),
        measurement.alive_tasks_before,
        measurement.alive_tasks_after
    );
}
