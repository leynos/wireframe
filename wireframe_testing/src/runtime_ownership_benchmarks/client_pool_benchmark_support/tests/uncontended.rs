//! Bounds on the uncontended fast path: spawns, waits, and the zero case.

use crate::runtime_ownership_benchmarks::client_pool_benchmark_support::{
    AcquisitionMode,
    PoolWorkload,
    measure_pool_acquisition,
};

#[tokio::test]
async fn zero_iterations_measures_nothing() {
    let measurement = measure_pool_acquisition(PoolWorkload::default(), 0)
        .await
        .expect("zero iterations must succeed");

    assert_eq!(measurement.acquisitions, 0);
    assert_eq!(measurement.scheduler_spawns, 0);
    assert_eq!(measurement.capacity_waits, 0);
    assert_eq!(measurement.construction, std::time::Duration::ZERO);
    assert_eq!(measurement.alive_task_delta(), 0);
}

/// Every lease release reaches `kick` on an idle scheduler, so a burst spawns.
///
/// This is the measurement that falsified the "uncontended spawns nothing"
/// assumption. It is recorded as a test rather than a comment so the corrected
/// contract is pinned: if a later refactor removes the per-release notify, this
/// fails and the baseline table must be revisited.
///
/// The lower bound is only sound because the measurement builds and warms its
/// own pool and closes it, so `kick` sees an idle scheduler at the burst's
/// start. `kick` spawns on a compare-exchange, not on a count, so a worker left
/// servicing by an earlier phase would make every release here lose the
/// exchange and the count legitimately zero. That is why this test asserts the
/// bound on a fresh pool rather than on a reused one, and why the shared
/// helper's doc states the precondition.
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
    assert!(
        !measurement.contended(),
        "one caller looping {acquisitions} times over a {pool_size}-slot pool never exhausts it, \
         so it must record no capacity waits; {} recorded",
        measurement.capacity_waits
    );
}

/// The waiting counter is discriminating in the other direction too.
///
/// Asserting only that a contended burst records waits would not show the
/// counter means anything — a counter that always increments would pass. The
/// uncontended cases above establish the zero, and this establishes the
/// non-zero, so together they pin the counter to the event it claims to count.
#[rstest::rstest]
#[case(1, 4)]
#[case(4, 16)]
#[case(16, 64)]
#[tokio::test]
async fn uncontended_acquisition_records_no_capacity_waits(
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
        measurement.capacity_waits, 0,
        "a single caller that releases before re-acquiring must never park on a {pool_size}-slot \
         pool"
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

    // The two readings bracket the acquisition, not the construction: the
    // `before` sample is taken once the pool is already built, so the eagerly
    // connected tasks sit on both sides of the subtraction and cancel. Pool
    // closure is not what makes this delta clean — it happens after the `after`
    // sample — and it could not be moved inside the bracket without measuring
    // teardown. `<= 0` is affordable here because `#[tokio::test]` defaults to a
    // current-thread runtime, where this gauge is deterministic; the same
    // assertion made on a multi-thread runtime would be reading the whole
    // runtime's scheduler.
    assert!(
        measurement.alive_task_delta() <= 0,
        "a closed pool must not leave construction tasks behind: delta {} on a {pool_size}-slot \
         pool (before {}, after {})",
        measurement.alive_task_delta(),
        measurement.alive_tasks_before,
        measurement.alive_tasks_after
    );
}
