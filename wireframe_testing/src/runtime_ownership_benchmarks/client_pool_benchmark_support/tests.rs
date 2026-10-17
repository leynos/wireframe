//! Unit coverage for pool workload labels, rejection rules, and the counters.

use std::time::Duration;

use super::{
    AcquisitionMode,
    PoolMeasurement,
    PoolWorkload,
    measure_contended_latency,
    measure_pool_acquisition,
};

/// Attempts allowed before a contended burst is reported as a failure.
///
/// The contended acquire path intermittently fails a burst with a spurious
/// `disconnected`. On the named baseline machine the post-#535 rate reaches
/// roughly one burst in 73 at 8 contenders on the FIFO policy. The cases below
/// assert what a contended burst *can* produce, so they retry a bounded number
/// of times rather than failing the suite on an interleaving that predates this
/// work and is recorded by issue #639 against issue #647. Eight attempts makes
/// a false failure vanishingly unlikely without hiding a burst that never
/// succeeds: a pool that failed every attempt is a different and much more
/// serious defect, and is reported as such.
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
    assert_eq!(measurement.capacity_waits, 0);
    assert_eq!(measurement.construction, Duration::ZERO);
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

/// A contended burst must be *observed* to contend, not assumed to.
///
/// The burst empties the pool and holds it empty while every contender asks for
/// capacity, so each contender must park: a contended workload with `c`
/// contenders records at least `c` waits before any capacity is returned. That
/// is a floor, not an equality — the contended rounds that follow routinely push
/// it higher — but it is enough to falsify a burst that never actually
/// contended, which is what a start barrier alone fails to rule out.
///
/// The spawn count is kept beside it as task-churn evidence. It is not evidence
/// of contention: every release notifies the scheduler whether or not anyone is
/// waiting.
#[rstest::rstest]
#[case(AcquisitionMode::ContendedFifo)]
#[case(AcquisitionMode::ContendedRoundRobin)]
#[tokio::test]
async fn contended_acquisition_observes_capacity_waits(#[case] mode: AcquisitionMode) {
    const CONTENDERS: usize = 16;
    let measurement = contended_burst_with_retry(PoolWorkload {
        pool_size: 4,
        mode,
        acquisitions: 8,
        contenders: CONTENDERS,
    })
    .await
    .expect("contended measurement failed in every retry attempt");

    assert!(
        measurement.capacity_waits >= u64::try_from(CONTENDERS).unwrap_or(u64::MAX),
        "a burst that empties a 4-slot pool and holds it empty must park every one of \
         {CONTENDERS} contenders, but {} waits were recorded",
        measurement.capacity_waits
    );
    assert!(
        measurement.contended(),
        "the measurement must classify itself as contended"
    );
    assert!(
        measurement.scheduler_spawns >= 1,
        "handing capacity to a parked waiter must enter the scheduler slow path at least once"
    );
    assert!(
        measurement.acquisitions >= 1,
        "contended contenders must still be granted leases"
    );
}

/// Every attempted burst lands in exactly one of the two buckets.
///
/// This is what makes the distribution readable: the sample count and the
/// failure count must sum to the number of attempts, so a reader can tell how
/// much of the run the statistics describe and how much they do not. The
/// property holds whether a burst succeeds or fails, so unlike the assertions
/// above it needs no retry — retrying here would be trying to steer an
/// observation the assertion does not depend on.
///
/// The sampling loop is deliberately not validated by asserting a sample count
/// of its own: on the recorded defect rate a run of this size usually succeeds
/// every burst, and an assertion that only holds on the usual path would be a
/// flake rather than a check.
#[tokio::test]
async fn sampled_bursts_account_for_every_attempt() {
    const SAMPLES: usize = 3;
    const CONTENDERS: usize = 16;
    let latency = measure_contended_latency(
        PoolWorkload {
            pool_size: 4,
            mode: AcquisitionMode::ContendedFifo,
            acquisitions: 8,
            contenders: CONTENDERS,
        },
        SAMPLES,
    )
    .await
    .expect("contended sampling failed");

    assert_eq!(
        latency.successes() + latency.failure_count(),
        SAMPLES,
        "every attempted burst must be recorded as a sample or as a failure; the retained \
         messages ({:?}) name the failures that were counted",
        latency.failures()
    );
}

/// The reported churn is the sampled run's, not the pool's lifetime total.
///
/// Every burst empties the pool and holds it empty while `contenders` callers
/// ask for capacity, so each one forces at least that many waits. Summing over
/// the run gives a floor that no per-burst reading can satisfy by itself, which
/// is what distinguishes a delta over the sampling window from a cumulative
/// counter that happens to be non-zero.
///
/// Scheduler spawns are reported beside it as task churn. They are deliberately
/// *not* asserted to a floor: `kick` spawns on every release whether or not
/// anyone is waiting, so a spawn count cannot witness contention. The waits can,
/// and they are what the floor uses.
#[tokio::test]
async fn sampled_bursts_report_this_runs_churn() {
    const SAMPLES: usize = 3;
    const CONTENDERS: usize = 16;
    let latency = measure_contended_latency(
        PoolWorkload {
            pool_size: 4,
            mode: AcquisitionMode::ContendedFifo,
            acquisitions: 8,
            contenders: CONTENDERS,
        },
        SAMPLES,
    )
    .await
    .expect("contended sampling failed");

    let floor = u64::try_from(SAMPLES.saturating_mul(CONTENDERS)).unwrap_or(u64::MAX);
    assert!(
        latency.churn().capacity_waits >= floor,
        "each of {SAMPLES} bursts parks {CONTENDERS} contenders, so the run must report at least \
         {floor} waits; {} were reported",
        latency.churn().capacity_waits
    );
    assert!(
        latency.churn().scheduler_spawns >= 1,
        "serving parked waiters must enter the scheduler's slow path at least once"
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
