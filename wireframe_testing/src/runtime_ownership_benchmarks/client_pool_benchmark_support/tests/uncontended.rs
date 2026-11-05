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

    assert_eq!(
        measurement.acquisitions, 0,
        "measurement.acquisitions should equal 0"
    );
    assert_eq!(
        measurement.scheduler_spawns, 0,
        "measurement.scheduler_spawns should equal 0"
    );
    assert_eq!(
        measurement.capacity_waits, 0,
        "measurement.capacity_waits should equal 0"
    );
    assert_eq!(
        measurement.construction,
        std::time::Duration::ZERO,
        "measurement.construction should equal std::time::Duration::ZERO"
    );
    assert_eq!(
        measurement.alive_task_delta(),
        0,
        "measurement.alive_task_delta() should equal 0"
    );
}

/// Acquisition is mailbox traffic to the pool's existing scheduler task.
///
/// The pool starts that task when it is constructed, which the measurement
/// bracket excludes, so the measured phase must report no spawns at all. This
/// pins the post-#647 contract: a later refactor that reintroduced a
/// per-release spawn would move this count and fail here.
#[rstest::rstest]
#[case(1, 1)]
#[case(1, 16)]
#[case(4, 16)]
#[case(16, 16)]
#[tokio::test]
async fn uncontended_acquisition_spawns_no_scheduler_tasks(
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
    assert_eq!(
        measurement.scheduler_spawns, 0,
        "a {acquisitions}-acquisition burst on a {pool_size}-slot pool must reuse the scheduler \
         task it started at construction"
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

/// A pool owns exactly one scheduler task for its whole life.
///
/// The measurement reports a delta, which cannot distinguish "no spawns" from
/// "one spawn at construction". This reads the pool's own counter, which is
/// never reset, so it proves the pool starts one task and that acquisition adds
/// none.
#[rstest::rstest]
#[case(1)]
#[case(4)]
#[case(16)]
#[tokio::test]
async fn a_pool_starts_exactly_one_scheduler_task(#[case] pool_size: usize) {
    let (_server, pool, _preamble) = wireframe::test_helpers::pool_client::build_preamble_pool(
        wireframe::client::ClientPoolConfig::default().pool_size(pool_size),
    )
    .await
    .expect("pool construction must succeed");

    let observer = pool.task_churn_observer();
    assert_eq!(
        observer.snapshot().scheduler_spawns,
        1,
        "constructing a {pool_size}-slot pool must start exactly one scheduler task"
    );

    let mut handle = pool.handle();
    for _ in 0..16 {
        let lease = handle
            .acquire()
            .await
            .expect("an uncontended acquire must succeed");
        drop(lease);
    }

    assert_eq!(
        observer.snapshot().scheduler_spawns,
        1,
        "sixteen acquisitions must not start another scheduler task"
    );
    pool.close().await;
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
    // connected tasks and the pool's persistent scheduler task sit on both sides
    // of the subtraction and cancel. `<= 0` is affordable here because
    // `#[tokio::test]` defaults to a current-thread runtime, where this gauge is
    // deterministic; the same assertion made on a multi-thread runtime would be
    // reading the whole runtime's scheduler.
    assert!(
        measurement.alive_task_delta() <= 0,
        "acquisition must not leave tasks behind: delta {} on a {pool_size}-slot pool (before {}, \
         after {})",
        measurement.alive_task_delta(),
        measurement.alive_tasks_before,
        measurement.alive_tasks_after
    );
}
