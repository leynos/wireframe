//! Contended-burst latency as a distribution, not a single figure.
//!
//! The contended acquire path both succeeds and fails, and the failures are not
//! rare enough to ignore: the live disconnect defect recorded in
//! `docs/runtime-ownership-baselines.md` fails roughly one burst in seventy at
//! eight contenders on the FIFO policy. A single elapsed value is therefore
//! ambiguous — it is either the duration of a burst that produced leases, or a
//! number for a run that produced none — and reporting it as "the contended
//! latency" would put a figure in the baseline whose meaning depends on which
//! case the sampler happened to hit.
//!
//! # Failures are counted, never retried into samples
//!
//! [`measure_contended_latency`] runs a fixed number of bursts. A burst that
//! fails is recorded as a failure and the loop moves on to the *next* burst; it
//! never re-runs the failed attempt and it never adds a timing for it. That
//! distinction is the whole point: a retry-until-success loop would report a
//! latency distribution whose samples were selected for having succeeded after
//! some number of tries, which is a biased sample of the fast path dressed up as
//! the population.
//!
//! A failure also has no meaningful duration. The burst that fails is the one
//! that waited for capacity it never received, so its elapsed time measures how
//! long the failure took to surface, not how long acquisition costs.
//!
//! # What each sample times
//!
//! One sample is one whole contended burst: exhaust the pool, park every
//! contender, release capacity, run the rounds, and let the task gauge settle.
//! That is deliberately the same quantity `measure_pool_acquisition` reports as
//! `elapsed`, bracketed the same way, so the distribution's median and the
//! single-iteration figure are comparable rather than two different things under
//! one name.
//!
//! # Why the churn is carried alongside
//!
//! The run also folds the pool's own counters into
//! [`ContendedLatency::churn`], because the timings alone do not say what the
//! regime cost. Issue #639 exists to record task churn before the ownership
//! refactors, and a latency distribution that dropped the counter delta would
//! answer only half of what the contended scope is asked. Keeping both in one
//! value means a caller cannot report the timings and forget the churn they
//! were bought with.

use std::time::Duration;

use wireframe::client::PoolTaskChurn;

use super::harness::PoolHarness;
use crate::runtime_ownership_benchmarks::client_pool_benchmark_support::workload::PoolWorkload;

/// How many failure messages a distribution retains verbatim.
///
/// The defect this exists to surface is intermittent, so a handful of distinct
/// messages is enough to identify it; the count is what carries the rate. A
/// pathological run that fails every burst must not accumulate a message per
/// burst in a value that is then reported.
const RETAINED_FAILURES: usize = 8;

/// The distribution of contended-burst latencies from one sampling run.
///
/// Successful samples and failed attempts are kept apart and reported apart.
/// Statistics are computed over the successful samples only, because a failure
/// has no acquisition latency to contribute; [`ContendedLatency::failure_count`]
/// is what says how much of the run the statistics do not describe. The churn
/// the run cost is carried beside both, on [`ContendedLatency::churn`].
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ContendedLatency {
    /// Successful burst durations, ascending, so percentiles are index lookups.
    samples: Vec<Duration>,
    /// How many bursts failed, including those whose message was not retained.
    failure_count: usize,
    /// The first [`RETAINED_FAILURES`] failure messages, in the order seen.
    failures: Vec<String>,
    /// Scheduler churn the sampled bursts cost, folded across the run.
    churn: PoolTaskChurn,
}

impl ContendedLatency {
    /// Record one successful sample.
    pub(super) fn push_sample(&mut self, elapsed: Duration) {
        // Ascending order maintained on insert: the run is short (tens of
        // samples) and this keeps the statistics functions free of any need to
        // sort a borrowed copy.
        let position = self
            .samples
            .partition_point(|existing| *existing <= elapsed);
        self.samples.insert(position, elapsed);
    }

    /// Record one failed burst, retaining its message only while there is room.
    pub(super) fn push_failure(&mut self, message: String) {
        self.failure_count = self.failure_count.saturating_add(1);
        if self.failures.len() < RETAINED_FAILURES {
            self.failures.push(message);
        }
    }

    /// Fold one burst's churn into the run's total.
    pub(super) const fn push_churn(&mut self, churn: PoolTaskChurn) {
        self.churn = self.churn.saturating_add(churn);
    }

    /// Bursts that produced a lease and contributed a sample.
    ///
    /// # Examples
    ///
    /// ```
    /// use wireframe_testing::runtime_ownership_benchmarks::ContendedLatency;
    ///
    /// assert_eq!(ContendedLatency::default().successes(), 0);
    /// ```
    #[must_use]
    pub const fn successes(&self) -> usize { self.samples.len() }

    /// Bursts that failed, whether or not their message was retained.
    ///
    /// # Examples
    ///
    /// ```
    /// use wireframe_testing::runtime_ownership_benchmarks::ContendedLatency;
    ///
    /// assert_eq!(ContendedLatency::default().failure_count(), 0);
    /// ```
    #[must_use]
    pub const fn failure_count(&self) -> usize { self.failure_count }

    /// The retained failure messages, at most [`RETAINED_FAILURES`] of them.
    ///
    /// # Examples
    ///
    /// ```
    /// use wireframe_testing::runtime_ownership_benchmarks::ContendedLatency;
    ///
    /// assert!(ContendedLatency::default().failures().is_empty());
    /// ```
    #[must_use]
    pub fn failures(&self) -> &[String] { &self.failures }

    /// Every successful sample, ascending.
    ///
    /// # Examples
    ///
    /// ```
    /// use wireframe_testing::runtime_ownership_benchmarks::ContendedLatency;
    ///
    /// assert!(ContendedLatency::default().samples().is_empty());
    /// ```
    #[must_use]
    pub fn samples(&self) -> &[Duration] { &self.samples }

    /// Scheduler churn the whole run cost, folded across every burst.
    ///
    /// This is the issue's task-churn evidence for the contended scope, and it
    /// is reported per run rather than per sample: the pool's counters are
    /// cumulative and this is their delta over the sampling window, so dividing
    /// it by the sample count would imply a per-burst precision the counters do
    /// not have. Failed bursts contribute to it too — churn is what a burst
    /// *did*, whether or not it produced a lease — which is why it is not the
    /// successes' total.
    ///
    /// # Examples
    ///
    /// ```
    /// use wireframe_testing::runtime_ownership_benchmarks::ContendedLatency;
    ///
    /// assert_eq!(ContendedLatency::default().churn().scheduler_spawns, 0);
    /// ```
    #[must_use]
    pub const fn churn(&self) -> PoolTaskChurn { self.churn }

    /// The fastest successful burst, or `None` if none succeeded.
    #[must_use]
    pub fn min(&self) -> Option<Duration> { self.samples.first().copied() }

    /// The slowest successful burst, or `None` if none succeeded.
    #[must_use]
    pub fn max(&self) -> Option<Duration> { self.samples.last().copied() }

    /// The median successful burst, by nearest rank.
    #[must_use]
    pub fn median(&self) -> Option<Duration> { self.percentile(1, 2) }

    /// The 90th-percentile successful burst, by nearest rank.
    ///
    /// Reported beside the median because a contended burst's cost is
    /// tail-shaped: most bursts are served promptly once capacity returns, and
    /// the ones that are not are what a caller experiences as a stall.
    #[must_use]
    pub fn p90(&self) -> Option<Duration> { self.percentile(9, 10) }

    /// The arithmetic mean of the successful bursts, or `None` if none.
    #[must_use]
    pub fn mean(&self) -> Option<Duration> {
        let total = self
            .samples
            .iter()
            .copied()
            .fold(Duration::ZERO, Duration::saturating_add);
        // `checked_div` rather than `/`: the divisor is a `u32`, and a
        // conversion that silently truncated would divide by the wrong number.
        total.checked_div(u32::try_from(self.samples.len()).ok()?)
    }

    /// Nearest-rank percentile: the sample at `ceil(n × numerator/denominator)`.
    ///
    /// Nearest rank rather than an interpolating definition because these are
    /// observed durations, and an interpolated value between two samples is a
    /// figure no burst actually took.
    fn percentile(&self, numerator: usize, denominator: usize) -> Option<Duration> {
        if self.samples.is_empty() {
            return None;
        }
        let rank = self
            .samples
            .len()
            .saturating_mul(numerator)
            .div_ceil(denominator);
        // Rank is one-based, so it is shifted to an index and clamped: the
        // ceiling can land one past the last sample when the multiplication
        // rounds up.
        let index = rank
            .saturating_sub(1)
            .min(self.samples.len().saturating_sub(1));
        self.samples.get(index).copied()
    }
}

/// Time `samples` contended bursts, keeping successes and failures apart.
///
/// The pool is built and warmed once, outside every timed burst, so no sample
/// carries a connection handshake — the same bracket
/// [`measure_pool_acquisition`](super::measure_pool_acquisition) applies, for
/// the same reason.
///
/// `samples` counts *attempts*, not successes: a run of twenty bursts that fails
/// three reports seventeen samples and a failure count of three. That is the
/// honest reading — the alternative, retrying until twenty bursts succeed,
/// discards exactly the evidence the defect exists to record.
///
/// # Errors
///
/// Returns a description of the invalid workload, the pool-construction failure,
/// the warm-up failure, or a task gauge that would not settle. Individual burst
/// failures are *not* errors: they are recorded on the returned distribution and
/// reported beside it.
pub async fn measure_contended_latency(
    workload: PoolWorkload,
    samples: usize,
) -> Result<ContendedLatency, String> {
    workload.validate()?;
    if samples == 0 {
        // Nothing is measured, so nothing needs a live peer. Answering before
        // the harness exists matches how `measure_pool_acquisition` treats its
        // own zero case, and keeps the empty distribution the same value
        // whether or not a pool could have been built.
        return Ok(ContendedLatency::default());
    }

    let harness = PoolHarness::build(workload).await?;
    let measured = harness.sample_bursts(samples).await;
    // Close before deciding what to report, so a sampling failure still tears
    // the pool down. The samples are already taken, so closing first cannot
    // change them.
    harness.close().await;
    measured
}

/// Burst timings a benchmark asks for when nothing else is specified.
///
/// Matches the Criterion sample budget the timing groups pin, so the contended
/// distribution and the timed groups rest on comparable evidence. It is an
/// attempt count rather than a success target; see [`measure_contended_latency`].
pub const CONTENDED_LATENCY_SAMPLES: usize = 20;

#[cfg(test)]
mod tests;
