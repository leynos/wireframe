//! Accounting over a sampled run: every attempt is recorded, and churn is
//! reported for the window rather than the pool's lifetime.

use crate::runtime_ownership_benchmarks::client_pool_benchmark_support::{
    AcquisitionMode,
    PoolWorkload,
    measure_contended_latency,
};

/// Every attempted burst lands in exactly one of the two buckets.
///
/// This is what makes the distribution readable: the sample count and the
/// failure count must sum to the number of attempts, so a reader can tell how
/// much of the run the statistics describe and how much they do not. The
/// property holds whether a burst succeeds or fails, so unlike the assertions
/// in `contended` it needs no retry — retrying here would be trying to steer an
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

    // Only successful bursts are floored. A failed burst is still counted in
    // `churn` — the scheduler work it cost is real — but it can fail before all
    // its contenders reach the forced-wait stage, so it cannot be assumed to
    // contribute `CONTENDERS` waits.
    let successes = latency.successes();
    assert!(
        successes > 0,
        "the floor below rests on successful bursts, and this run produced none of {SAMPLES}: a \
         burst that never succeeded parks no contenders at all, so the assertion would be vacuous"
    );
    let floor = u64::try_from(successes.saturating_mul(CONTENDERS)).unwrap_or(u64::MAX);
    assert!(
        latency.churn().capacity_waits >= floor,
        "each of {successes} successful bursts parks {CONTENDERS} contenders, so the run must \
         report at least {floor} waits; {} were reported",
        latency.churn().capacity_waits
    );
    assert!(
        latency.churn().scheduler_spawns >= 1,
        "serving parked waiters must enter the scheduler's slow path at least once"
    );
}
