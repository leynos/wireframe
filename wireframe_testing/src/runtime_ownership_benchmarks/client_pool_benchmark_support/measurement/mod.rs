//! Driving one pool workload and reporting its counts and durations.
//!
//! The two regimes are separate modules — [`phase`] owns the acquisition phase,
//! [`contended`] the racing burst — because they are genuinely different control
//! flows and a shared body would blur which one a regression sits in. This
//! module owns the bracket around them: it builds and warms the pool, hands the
//! pool to a phase, and reports the acquisition time separately from the
//! construction time.
//!
//! # The bracket
//!
//! Construction and teardown are outside the timed and counted windows; the
//! acquire/release rounds are inside them. That boundary is the whole point of
//! this module. Building the pool per iteration put `pool_size` eager loopback
//! connections inside the timed region, so the reported duration was dominated
//! by TCP handshakes rather than by the acquire path the baseline is about, and
//! the allocation window counted connection setup alongside the acquire path.
//!
//! [`PoolHarness`] is what makes the boundary hold: it builds and warms a pool
//! and is *passed* to the measuring code, so nothing inside a window can
//! construct anything. Construction cost is still reported, as its own figure on
//! [`PoolMeasurement::construction`], because it is a real cost of the scope —
//! it just is not the cost this scope exists to measure.
//!
//! # Warm-up
//!
//! The first acquisition after construction is not reported: it pays for lazy
//! slot checkout and for the scheduler's first idle-to-busy transition, costs
//! later iterations do not repeat. Warming the pool before the timed region
//! keeps iteration one comparable with iteration ten.

mod contended;
mod distribution;
mod harness;
mod phase;

use std::time::Duration;

pub use contended::CONTENDED_BURST_TIMEOUT;
pub use distribution::{CONTENDED_LATENCY_SAMPLES, ContendedLatency, measure_contended_latency};
pub use harness::PoolHarness;
use wireframe::client::ClientError;

use super::workload::{PoolMeasurement, PoolWorkload};

/// Acquisitions used to warm a freshly built pool before it is measured.
///
/// One is enough to force the work that only happens once: the first slot
/// checkout, and the first idle-to-busy transition of the scheduler. A second
/// would add its own churn to no purpose.
pub(super) const WARMUP_ACQUISITIONS: usize = 1;

/// Live task count for the current runtime.
///
/// `RuntimeMetrics::num_alive_tasks` is gated only on 64-bit atomics, so it is
/// available without the `rt-metrics` feature or `tokio_unstable`.
pub(super) fn alive_tasks() -> usize {
    tokio::runtime::Handle::current()
        .metrics()
        .num_alive_tasks()
}

/// Yield until the live task gauge holds steady, or the attempt budget runs out.
///
/// A single `yield_now` is not enough to let a just-woken worker exit on its
/// first poll. Under a busy runtime the worker may not reach the head of its
/// queue by the time the gauge is read, so the count is taken while the task is
/// still alive and the settled delta comes out inflated. That made the caller's
/// bound load-sensitive: the same code passed in isolation and failed under the
/// full suite's concurrency.
///
/// Requiring the reading to hold steady across [`STABLE_POLLS`] consecutive
/// polls, rather than merely stopping its fall, is what makes it a settled state
/// rather than a race. A single flat pair of reads is not evidence of
/// quiescence: an exiting worker waiting to be polled produces exactly that.
///
/// `RuntimeMetrics::num_alive_tasks` counts every task on the runtime, not just
/// this workload's, so the caller must also ensure nothing else is running on
/// that runtime. See `docs/runtime-ownership-baselines.md`.
pub(super) async fn settled_alive_tasks() -> Result<usize, String> {
    /// Consecutive unchanged polls required before a reading counts as settled.
    ///
    /// More than one because a single flat pair can be a worker that is queued
    /// to exit but has not been polled yet. Small, because each poll is a full
    /// yield and the pool is idle here; the cost is a few scheduler quanta.
    const STABLE_POLLS: usize = 4;

    /// Total yields spent waiting for the gauge to settle.
    ///
    /// Generous relative to the handful of workers a burst can leave exiting,
    /// because a yield under load can cost a full scheduler quantum. Exhausting
    /// it is reported, not tolerated: a gauge that has not settled is exactly
    /// the condition that made this bound flaky.
    const SETTLE_ATTEMPTS: usize = 256;

    let mut previous = alive_tasks();
    let mut current = previous;
    let mut stable = 0;
    for _ in 0..SETTLE_ATTEMPTS {
        tokio::task::yield_now().await;
        current = alive_tasks();
        if current == previous {
            stable += 1;
        } else {
            stable = 0;
        }
        previous = current;
        if stable >= STABLE_POLLS {
            return Ok(current);
        }
    }

    Err(format!(
        "live task gauge did not hold steady for {STABLE_POLLS} polls within {SETTLE_ATTEMPTS} \
         yields; it last read {current}, so the settled delta cannot be reported"
    ))
}

/// Build the pool configuration for a workload.
pub(super) fn pool_config(workload: PoolWorkload) -> wireframe::client::ClientPoolConfig {
    workload
        .mode
        .configure(wireframe::client::ClientPoolConfig::default().pool_size(workload.pool_size))
}

/// Acquire and immediately release `rounds` leases, counting the grants.
pub(super) async fn acquire_round(
    handle: &mut wireframe::client::PoolHandle<
        wireframe::serializer::BincodeSerializer,
        wireframe::test_helpers::pool_client::ClientHello,
        (),
    >,
    rounds: usize,
) -> Result<u64, ClientError> {
    let mut granted = 0u64;
    for _ in 0..rounds {
        let lease = handle.acquire().await?;
        granted += 1;
        // Releasing before the next acquire keeps the pool from being starved
        // by this caller, which is what makes the uncontended regime uncontended.
        drop(lease);
    }
    Ok(granted)
}

/// Time `iterations` acquisition phases against one warm pool.
///
/// A thin wrapper over [`PoolHarness`] for callers that want nothing more than
/// a finished measurement. Any caller that also counts allocations should take
/// the harness directly and hold it across its own window, so construction sits
/// outside it.
///
/// # Errors
///
/// Returns a description of the invalid workload, pool-construction failure,
/// warm-up failure, or acquisition failure. A contended workload whose
/// contender count does not exceed its pool size is rejected, because it could
/// not enter the scheduler's slow path and would report an uncontended finding
/// under a contended label.
///
/// # Examples
///
/// ```rust,no_run
/// use wireframe_testing::runtime_ownership_benchmarks::{
///     AcquisitionMode,
///     PoolWorkload,
///     measure_pool_acquisition,
/// };
///
/// # async fn example() -> Result<(), String> {
/// let measurement = measure_pool_acquisition(
///     PoolWorkload {
///         pool_size: 4,
///         mode: AcquisitionMode::Uncontended,
///         acquisitions: 16,
///         contenders: 16,
///     },
///     1,
/// )
/// .await?;
/// assert_eq!(measurement.acquisitions, 16);
/// # Ok(())
/// # }
/// ```
pub async fn measure_pool_acquisition(
    workload: PoolWorkload,
    iterations: u64,
) -> Result<PoolMeasurement, String> {
    workload.validate()?;
    if iterations == 0 {
        // No pool is built at all, so there is no construction cost to report
        // and no churn to attribute. Answering before the harness exists also
        // keeps the zero case free of a live peer.
        return Ok(PoolMeasurement {
            acquisitions: 0,
            elapsed: Duration::ZERO,
            construction: Duration::ZERO,
            scheduler_spawns: 0,
            capacity_waits: 0,
            alive_tasks_before: alive_tasks(),
            alive_tasks_after: alive_tasks(),
        });
    }

    let harness = PoolHarness::build(workload).await?;
    let measured = harness.measure(iterations).await;

    // Close before deciding what to report, so an unsuccessful run still tears
    // its pool down. The measurement is already complete, so closing first
    // cannot affect it.
    harness.close().await;
    measured
}
