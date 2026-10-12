//! A pool built and warmed *before* the window that measures it.
//!
//! Construction and warm-up are the cost of getting a pool ready to be
//! measured, not the cost the measurement is about. Separating them into an
//! owned value is what lets a caller pay that cost outside a timing or
//! allocation window and then measure only the acquire/release rounds.
//!
//! The separation matters most for the allocation baseline. A counting window
//! that includes construction records the allocations of `pool_size` eager
//! loopback connections alongside the acquire path's, and the pool-size term
//! swamps the signal the window exists to capture. Reusing one harness across
//! iterations removes it.
//!
//! Construction cost is not discarded, only relocated: it is carried on
//! [`PoolMeasurement::construction`] so a caller still has the figure.

use std::time::{Duration, Instant};

use wireframe::{
    client::{PoolTaskChurn, PoolTaskChurnObserver},
    test_helpers::pool_client::PoolTestServer,
};

use super::{
    WARMUP_ACQUISITIONS,
    acquire_round,
    alive_tasks,
    contended::contended_round_bounded,
    distribution::ContendedLatency,
    phase,
    phase::PhaseOutcome,
    pool_config,
    settled_alive_tasks,
};
use crate::runtime_ownership_benchmarks::client_pool_benchmark_support::workload::{
    PoolMeasurement,
    PoolWorkload,
};

/// A pool that is already built and warmed, ready to be measured.
///
/// Build once, measure as many times as needed, then close. Every `measure`
/// call reuses the same physical connections, so no iteration pays for a
/// handshake.
pub struct PoolHarness {
    /// Loopback peer the pool's sockets are connected to.
    ///
    /// Held for as long as the pool rather than dropped after construction.
    /// Dropping it stops the accept loop, so a slot that had to open a fresh
    /// connection — after an idle recycle, say — would fail. Nothing in a
    /// measurement reconnects today, but tying the peer's lifetime to the
    /// pool's keeps that a property of the types instead of a coincidence of
    /// the current timings.
    _server: PoolTestServer,
    /// The pool every measurement runs against.
    pool: phase::TestPool,
    /// Read-only view of the pool's task-churn counters.
    observer: PoolTaskChurnObserver,
    /// Churn the warm-up produced, which the totals carry.
    warmup_churn: PoolTaskChurn,
    /// How long construction took, reported rather than measured against.
    construction: Duration,
    /// The workload this pool was configured for.
    ///
    /// Stored so a caller cannot measure a pool against workload parameters it
    /// was not built with; the two would silently disagree.
    workload: PoolWorkload,
}

impl PoolHarness {
    /// Build and warm a pool for `workload`.
    ///
    /// The returned harness has already paid for construction and for the first
    /// acquisition, so the first `measure` call is comparable with the last.
    ///
    /// # Errors
    ///
    /// Returns a description of the invalid workload, of the failed connection,
    /// of a warm-up acquisition that failed, or of a task gauge that would not
    /// settle afterwards.
    pub async fn build(workload: PoolWorkload) -> Result<Self, String> {
        workload.validate()?;

        let started = Instant::now();
        let (server, pool, _preamble) =
            wireframe::test_helpers::pool_client::build_preamble_pool(pool_config(workload))
                .await
                .map_err(|err| format!("failed to build pooled client: {err}"))?;
        let construction = started.elapsed();

        let observer = pool.task_churn_observer();
        let before_warmup = observer.snapshot();
        let mut warmup_handle = pool.handle();
        acquire_round(&mut warmup_handle, WARMUP_ACQUISITIONS)
            .await
            .map_err(|err| format!("pool warm-up acquire failed: {err}"))?;
        // Let any worker the warm-up woke exit before the measured phase starts,
        // so its exit is not charged to the first measured iteration. The gauge
        // is sampled after that settle, which is what makes the seed's opening
        // reading comparable with the first phase's closing one.
        settled_alive_tasks().await?;
        let warmup_churn = observer.snapshot().since(before_warmup);

        Ok(Self {
            _server: server,
            pool,
            observer,
            warmup_churn,
            construction,
            workload,
        })
    }

    /// Time `iterations` acquisition phases against this already-warm pool.
    ///
    /// Nothing here constructs anything: the elapsed time covers the
    /// acquire/release rounds alone, which is what the scope exists to report.
    ///
    /// # Errors
    ///
    /// Returns a description of the first acquisition phase that failed. Later
    /// iterations do not run once one has failed, but the pool stays open so
    /// the caller can still close it.
    pub async fn measure(&self, iterations: u64) -> Result<PoolMeasurement, String> {
        let started = Instant::now();
        let mut totals = PhaseOutcome::seed(0, self.warmup_churn, alive_tasks());
        let mut outcome = Ok(());
        for _ in 0..iterations {
            match phase::acquisition_phase(self.workload, &self.pool, &self.observer).await {
                Ok(phase) => totals = phase.accumulate(totals),
                Err(error) => {
                    outcome = Err(error);
                    break;
                }
            }
        }
        let elapsed = started.elapsed();

        outcome?;
        Ok(phase::finish(totals, elapsed, self.construction))
    }

    /// Time `samples` contended bursts individually, keeping failures apart.
    ///
    /// Each burst is bracketed on its own, so the distribution's entries are
    /// what a caller experiences as one acquisition burst rather than an
    /// average over many. The bracket matches [`PoolHarness::measure`]'s — pool
    /// construction and warm-up outside, the burst inside — so the two figures
    /// are comparable.
    ///
    /// A failed burst is recorded and the loop continues. It is deliberately
    /// not retried: a retry would replace the failure with a freshly timed
    /// success, and the failure count is one of the two things this exists to
    /// report.
    ///
    /// The pool's counters are read around each burst as well as the clock, so
    /// the returned churn is this sampling run's delta rather than the counters'
    /// value since the pool was built. A failed burst's churn is included: the
    /// scheduler work a failure cost is still work the regime performed.
    ///
    /// # Errors
    ///
    /// Returns a description of the invalid workload, or of a task gauge that
    /// would not settle between bursts. Individual burst failures are recorded
    /// on the returned distribution rather than propagated.
    pub async fn sample_bursts(&self, samples: usize) -> Result<ContendedLatency, String> {
        let mut latency = ContendedLatency::default();
        for _ in 0..samples {
            let churn_before = self.observer.snapshot();
            let started = Instant::now();
            let outcome = contended_round_bounded(self.workload, &self.pool, &self.observer).await;
            // Timed before classifying, so a failure's duration is available
            // for diagnostics even though it contributes no sample.
            let elapsed = started.elapsed();
            latency.push_churn(self.observer.snapshot().since(churn_before));
            match outcome {
                Ok(_) => latency.push_sample(elapsed),
                Err(error) => latency.push_failure(error),
            }
            // Settle between bursts for the same reason the phase bracket does:
            // a worker the released capacity woke is still being polled, and
            // the next burst must not inherit its exit.
            settled_alive_tasks().await?;
        }
        Ok(latency)
    }

    /// Close the pool and drop its warm sockets.
    pub async fn close(self) { self.pool.close().await; }
}
