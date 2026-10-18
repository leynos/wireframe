//! The contended slow path: rejection rules, observed waits, and churn.

use crate::runtime_ownership_benchmarks::client_pool_benchmark_support::{
    AcquisitionMode,
    PoolMeasurement,
    PoolWorkload,
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
/// that [`CONTENDED_BURST_TIMEOUT`](super::super::CONTENDED_BURST_TIMEOUT)
/// converts into an error — surface as `Err`, so the retry does not inspect
/// error text. Returns the first successful observation, or the last error if
/// every attempt failed.
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
