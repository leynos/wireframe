//! One acquisition phase against an already-built, already-warm pool.
//!
//! The pool's lifetime is the point of this module. Construction happens
//! *before* this module is entered and teardown *after* it returns, so neither
//! is inside any window the caller measures. That placement was forced by
//! measurement rather than chosen for tidiness.
//!
//! An earlier revision built and closed a pool per iteration, inside the timed
//! region. That measured pool construction — a `bb8` pool that connects eagerly
//! through a real loopback socket — as though it were acquisition cost, and the
//! resulting figure was dominated by `pool_size` TCP handshakes rather than by
//! the acquire/release path under study. The counters were bracketed correctly
//! even then; only the timer was wrong, which is exactly the kind of error a
//! baseline must not carry forward.

use std::{hint::black_box, time::Duration};

use wireframe::client::{PoolHandle, PoolTaskChurn, PoolTaskChurnObserver, WireframeClientPool};

use super::{acquire_round, contended::contended_round_bounded, settled_alive_tasks};
use crate::runtime_ownership_benchmarks::client_pool_benchmark_support::workload::{
    PoolMeasurement,
    PoolWorkload,
};

/// The pooled client type every workload here drives.
pub(super) type TestPool = WireframeClientPool<
    wireframe::serializer::BincodeSerializer,
    wireframe::test_helpers::pool_client::ClientHello,
    (),
>;

/// The handle type a caller acquires leases from.
pub(super) type TestHandle = PoolHandle<
    wireframe::serializer::BincodeSerializer,
    wireframe::test_helpers::pool_client::ClientHello,
    (),
>;

/// Everything one acquisition phase observed.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(super) struct PhaseOutcome {
    /// Leases granted during the phase.
    pub(super) granted: u64,
    /// Scheduler and capacity-wait churn the phase accumulated.
    pub(super) churn: PoolTaskChurn,
    /// Live tasks immediately before the phase.
    pub(super) alive_before: usize,
    /// Live tasks once the phase has settled.
    pub(super) alive_after: usize,
}

impl PhaseOutcome {
    /// Fold this phase into a running total, keeping the bracketed task gauge.
    ///
    /// Iterations share one pool, so a caller measuring several of them wants
    /// the churn summed — it all happened to the same pool — while the task
    /// gauge is a level rather than a count and is only meaningful for the
    /// phases actually bracketed. This phase's `alive_after` replaces the
    /// running one; the running `alive_before` is kept, so the pair spans every
    /// phase the caller ran.
    pub(super) const fn accumulate(self, running: Self) -> Self {
        Self {
            granted: self.granted,
            churn: running.churn.saturating_add(self.churn),
            alive_before: running.alive_before,
            alive_after: self.alive_after,
        }
    }

    /// Seed an accumulator before the first phase, carrying prior churn.
    ///
    /// The churn argument is the work already done to this pool — construction
    /// and warm-up — which belongs in the totals because it happened to the same
    /// pool, even though it is deliberately outside every timed window.
    pub(super) const fn seed(granted: u64, churn: PoolTaskChurn, alive_before: usize) -> Self {
        Self {
            granted,
            churn,
            alive_before,
            alive_after: alive_before,
        }
    }
}

/// Run one acquisition phase against a pool that is already built and warm.
///
/// The pool is borrowed rather than created, so nothing in this function pays
/// for construction. An uncontended phase loops the caller's handle; a
/// contended phase runs the bounded racing burst. Both finish by waiting for
/// the live-task gauge to settle, because a worker spawned by the final release
/// observes an empty queue and exits on its first poll — reading the gauge
/// before that happens would report a task already on its way out.
///
/// # Errors
///
/// Returns a description of the acquisition failure, or of a gauge that would
/// not settle. A contended burst that stalls is reported by
/// [`contended_round_bounded`] after
/// [`CONTENDED_BURST_TIMEOUT`](super::CONTENDED_BURST_TIMEOUT) rather than
/// hanging.
pub(super) async fn acquisition_phase(
    workload: PoolWorkload,
    pool: &TestPool,
    observer: &PoolTaskChurnObserver,
) -> Result<PhaseOutcome, String> {
    let churn_before = observer.snapshot();
    let alive_before = super::alive_tasks();

    let granted = if workload.mode.is_contended() {
        contended_round_bounded(workload, pool, observer).await?
    } else {
        let mut handle = pool.handle();
        acquire_round(&mut handle, workload.acquisitions)
            .await
            .map_err(|err| format!("uncontended acquire failed: {err}"))?
    };

    // Read the gauge only after the acquisition has succeeded. After a stall it
    // is not expected to hold steady, and the settle wait would fail for a
    // reason that only restates the stall. The churn snapshot is taken after the
    // same wait, so any worker the settle saw exit has already been counted.
    let alive_after = settled_alive_tasks().await?;

    Ok(PhaseOutcome {
        granted,
        churn: observer.snapshot().since(churn_before),
        alive_before,
        alive_after,
    })
}

/// Package the accumulated phases into the measurement a caller receives.
///
/// The two durations stay separate because they answer different questions.
/// `elapsed` is the cost of the acquire/release rounds, which is what this scope
/// measures; `construction` is what it cost to get the pool into a state where
/// those rounds could run. Folding the second into the first would put
/// `pool_size` eager loopback handshakes back inside the figure the scope exists
/// to report.
pub(super) fn finish(
    outcome: PhaseOutcome,
    elapsed: Duration,
    construction: Duration,
) -> PoolMeasurement {
    PoolMeasurement {
        acquisitions: black_box(outcome.granted),
        elapsed,
        construction,
        scheduler_spawns: outcome.churn.scheduler_spawns,
        capacity_waits: outcome.churn.capacity_waits,
        alive_tasks_before: outcome.alive_before,
        alive_tasks_after: outcome.alive_after,
    }
}
