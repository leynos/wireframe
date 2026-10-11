//! Unit coverage for pool workload labels, rejection rules, and the counters.

use super::{AcquisitionMode, PoolMeasurement, PoolWorkload, measure_pool_acquisition};

/// Attempts allowed before a contended burst is reported as a failure.
///
/// The contended acquire path intermittently fails a burst with a spurious
/// `disconnected`, measured at roughly one burst in 913 at 16 contenders on the
/// named baseline machine. The cases below assert what a contended burst *can*
/// produce, so they retry a bounded number of times rather than failing the
/// suite on an interleaving that predates this work and is recorded by issue
/// #639 against issue #647. Eight attempts makes a false failure vanishingly
/// unlikely without hiding a burst that never succeeds: a pool that failed
/// every attempt is a different and much more serious defect, and is reported
/// as such.
const CONTENDED_BURST_ATTEMPTS: usize = 8;

/// Run one contended burst, retrying the acquire path's known spurious failures.
///
/// Both recorded failure modes — the spurious `disconnected`, and the stall
/// that [`CONTENDED_BURST_TIMEOUT`](super::CONTENDED_BURST_TIMEOUT) converts
/// into an error — surface as `Err`, so the retry does not inspect error text.
/// Returns the first successful observation, or the last error if every attempt
/// failed.
///
/// The error is accumulated with `unwrap_or` rather than `expect`: this helper
/// lives outside any `#[test]`, so the workspace's test-only `expect_used`
/// allowance does not cover it.
async fn contended_burst_with_retry(workload: PoolWorkload) -> Result<PoolMeasurement, String> {
    let mut last_error = None;
    for _ in 0..CONTENDED_BURST_ATTEMPTS {
        match measure_pool_acquisition(workload, 1).await {
            Ok(measurement) => return Ok(measurement),
            Err(error) => last_error = Some(error),
        }
    }

    Err(format!(
        "contended burst failed in all {CONTENDED_BURST_ATTEMPTS} attempts; last: {}",
        last_error.as_deref().unwrap_or("no error recorded")
    ))
}

/// Each regime's label names the parameter that actually drives it.
///
/// A contended burst is not driven by [`PoolWorkload::acquisitions`] — the
/// measurement path never reads it — so a contended label that reported
/// `acquisitions_256` would advertise a parameter the run did not honour. The
/// label states `rounds_{CONTENDED_ROUNDS_PER_CONTENDER}` instead, and this
/// test is what stops the two regimes from silently drifting back together.
#[rstest::rstest]
#[case(AcquisitionMode::Uncontended, "acquisitions_256")]
#[case(AcquisitionMode::ContendedFifo, "rounds_8")]
#[case(AcquisitionMode::ContendedRoundRobin, "rounds_8")]
fn labels_name_the_parameter_each_regime_honours(
    #[case] mode: AcquisitionMode,
    #[case] expected_fragment: &str,
) {
    let label = PoolWorkload {
        pool_size: 4,
        mode,
        acquisitions: 256,
        contenders: 16,
    }
    .label();

    assert!(
        label.contains(expected_fragment),
        "a {mode:?} label must report {expected_fragment}: {label}"
    );
    if mode.is_contended() {
        assert!(
            !label.contains("acquisitions_"),
            "a contended label must not advertise the acquisitions field it ignores: {label}"
        );
    }
}

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
    let measurement = contended_burst_with_retry(PoolWorkload {
        pool_size: 4,
        mode,
        acquisitions: 8,
        contenders: 16,
    })
    .await
    .expect("contended measurement failed in every retry attempt");

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
