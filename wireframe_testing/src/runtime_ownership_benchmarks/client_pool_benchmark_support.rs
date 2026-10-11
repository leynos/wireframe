//! Workloads and measurement for client pool acquisition.
//!
//! Pool acquisition has two regimes the issue asks to separate. An
//! **uncontended** acquire finds a free slot on the fast path and parks no
//! waiter, so the pool never needs a worker to hand capacity on. A
//! **contended** acquire exhausts the pool, parks waiters, and enters the
//! scheduler slow path.
//!
//! # Measured task churn, and why the obvious claim is false
//!
//! It is tempting to assert that uncontended acquisition spawns no scheduler
//! task. Measurement shows otherwise, and the numbers are recorded rather than
//! asserted away. `PooledClientLease::drop` unconditionally calls
//! `notify_capacity_available` (`src/client/pool/lease.rs`), so *every* release
//! reaches `PoolScheduler::kick`, and `try_acquire_immediately` arms the lease
//! with a release handle even on the fast path
//! (`src/client/pool/client_pool.rs`). `kick` then wins its
//! idle-to-busy compare-exchange and spawns a worker that immediately finds an
//! empty queue and exits.
//!
//! So the observable contract is **bounded**, not zero: a burst spawns at least
//! one worker and at most one per idle-to-busy transition. Measured deltas for
//! acquisition counts of 1, 16, and 256 were 1, 1, and 2 respectively, across
//! pool sizes 1, 4, and 16. That per-release churn is a genuine cost of the
//! current ownership topology and is exactly the kind of finding issue #639
//! exists to record before #637/#638 change it.
//!
//! # What the counters are compared against
//!
//! Because a non-zero delta is expected on both paths, the uncontended case is
//! not a control for the counter itself. The evidence that the counter can move
//! is the *contended* case, which forces repeated entry to the slow path; the
//! uncontended case bounds how far the fast path is allowed to drift. Both
//! report [`pool_scheduler_task_spawns`] and the live task counts.
//!
//! # What is bracketed
//!
//! Pool construction is excluded from both counters. `bb8` connects eagerly, so
//! building a pool of size N leaves N connection tasks alive; measuring the
//! task gauge across construction would report `pool_size` as acquisition
//! churn. See `pool_once`.
//!
//! # Transport
//!
//! Pool construction connects eagerly through `bb8`, and the crate exposes no
//! in-memory pool constructor, so this scenario needs a live peer. It uses the
//! loopback `PoolTestServer` (`src/test_helpers/pool_client.rs`), which is the
//! issue's documented transport-I/O exception. The peer only answers the
//! connection preamble; no frames are
//! exchanged during a measurement, so the timed work is acquisition and release
//! rather than request round-trips.

use std::{
    hint::black_box,
    sync::{
        Arc,
        Mutex,
        atomic::{AtomicUsize, Ordering},
    },
    time::{Duration, Instant},
};

use tokio::sync::Barrier;
use wireframe::{
    client::{ClientError, PoolFairnessPolicy, pool_scheduler_task_spawns},
    test_helpers::pool_client::build_preamble_pool,
};

/// Pool sizes the issue asks to cover.
pub const POOL_SIZES: [usize; 3] = [1, 4, 16];

/// Acquisitions used by the benchmark's default uncontended workload.
pub const DEFAULT_ACQUISITIONS: usize = 256;

/// Contenders used by the benchmark's default contended workload.
pub const DEFAULT_CONTENDERS: usize = 16;

/// Acquire/release rounds performed by each contender.
const CONTENDED_ROUNDS: usize = 8;

/// Which acquisition regime a workload exercises.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AcquisitionMode {
    /// One caller acquiring and releasing without exhausting the pool.
    Uncontended,
    /// Several callers racing, with the FIFO fairness policy.
    ContendedFifo,
    /// Several callers racing, with the round-robin fairness policy.
    ContendedRoundRobin,
}

impl AcquisitionMode {
    /// Short identifier used in benchmark labels.
    ///
    /// # Examples
    ///
    /// ```
    /// use wireframe_testing::runtime_ownership_benchmarks::AcquisitionMode;
    ///
    /// assert_eq!(AcquisitionMode::Uncontended.label(), "uncontended");
    /// ```
    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Self::Uncontended => "uncontended",
            Self::ContendedFifo => "contended_fifo",
            Self::ContendedRoundRobin => "contended_round_robin",
        }
    }

    /// Whether this mode races multiple callers.
    ///
    /// # Examples
    ///
    /// ```
    /// use wireframe_testing::runtime_ownership_benchmarks::AcquisitionMode;
    ///
    /// assert!(!AcquisitionMode::Uncontended.is_contended());
    /// assert!(AcquisitionMode::ContendedFifo.is_contended());
    /// ```
    #[must_use]
    pub const fn is_contended(self) -> bool { !matches!(self, Self::Uncontended) }

    /// The fairness policy this mode configures, or `None` when uncontended.
    ///
    /// # Examples
    ///
    /// ```
    /// use wireframe_testing::runtime_ownership_benchmarks::AcquisitionMode;
    ///
    /// assert_eq!(AcquisitionMode::Uncontended.policy(), None);
    /// ```
    #[must_use]
    pub const fn policy(self) -> Option<PoolFairnessPolicy> {
        match self {
            Self::Uncontended => None,
            Self::ContendedFifo => Some(PoolFairnessPolicy::Fifo),
            Self::ContendedRoundRobin => Some(PoolFairnessPolicy::RoundRobin),
        }
    }

    /// Contenders this mode runs.
    ///
    /// The contended modes always run at least two contenders, because a single
    /// caller cannot race itself into the scheduler's slow path.
    const fn contenders(self, requested: usize) -> usize {
        match self {
            Self::Uncontended => 1,
            Self::ContendedFifo | Self::ContendedRoundRobin => {
                if requested < 2 {
                    2
                } else {
                    requested
                }
            }
        }
    }
}

/// One pool-acquisition scenario.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PoolWorkload {
    /// Number of sockets the pool is configured to hold.
    pub pool_size: usize,
    /// Acquisition regime to exercise.
    pub mode: AcquisitionMode,
    /// Acquire/release cycles in the uncontended regime.
    pub acquisitions: usize,
    /// Contending callers in the contended regimes.
    pub contenders: usize,
}

impl Default for PoolWorkload {
    fn default() -> Self {
        Self {
            pool_size: 4,
            mode: AcquisitionMode::Uncontended,
            acquisitions: DEFAULT_ACQUISITIONS,
            contenders: DEFAULT_CONTENDERS,
        }
    }
}

impl PoolWorkload {
    /// Benchmark label embedding every recorded parameter.
    ///
    /// # Examples
    ///
    /// ```
    /// use wireframe_testing::runtime_ownership_benchmarks::{AcquisitionMode, PoolWorkload};
    ///
    /// let workload = PoolWorkload {
    ///     pool_size: 4,
    ///     mode: AcquisitionMode::Uncontended,
    ///     acquisitions: 256,
    ///     contenders: 16,
    /// };
    /// assert_eq!(
    ///     workload.label(),
    ///     "uncontended_pool_4_acquisitions_256_contenders_1"
    /// );
    /// ```
    #[must_use]
    pub fn label(self) -> String {
        format!(
            "{}_pool_{}_acquisitions_{}_contenders_{}",
            self.mode.label(),
            self.pool_size,
            self.acquisitions,
            self.mode.contenders(self.contenders)
        )
    }

    /// Reject workloads whose regime cannot produce the documented outcome.
    fn validate(self) -> Result<(), String> {
        if self.pool_size == 0 {
            return Err("pool_size must be at least 1".to_string());
        }
        if self.mode.is_contended() && self.mode.contenders(self.contenders) <= self.pool_size {
            return Err(format!(
                "mode {} needs more contenders than pool slots to contend: {} contenders, \
                 pool_size {}",
                self.mode.label(),
                self.contenders,
                self.pool_size
            ));
        }
        Ok(())
    }
}

/// Result of measuring one pool workload.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PoolMeasurement {
    /// Acquisitions completed (granted leases) in the final iteration.
    pub acquisitions: u64,
    /// Wall-clock time across every iteration.
    pub elapsed: Duration,
    /// Scheduler worker tasks spawned, relative to before the measurement.
    pub scheduler_spawns: u64,
    /// Pool-construction tasks live before the acquisition phase began.
    ///
    /// Sampled *after* the pool is built, so the eager bb8 connection tasks are
    /// already running and are excluded from the delta below.
    pub alive_tasks_before: usize,
    /// Live runtime tasks observed after the acquisition phase.
    pub alive_tasks_after: usize,
}

impl PoolMeasurement {
    /// Change in live task count across the measurement.
    ///
    /// # Examples
    ///
    /// ```
    /// use std::time::Duration;
    ///
    /// use wireframe_testing::runtime_ownership_benchmarks::PoolMeasurement;
    ///
    /// let measurement = PoolMeasurement {
    ///     acquisitions: 0,
    ///     elapsed: Duration::ZERO,
    ///     scheduler_spawns: 0,
    ///     alive_tasks_before: 2,
    ///     alive_tasks_after: 2,
    /// };
    /// assert_eq!(measurement.alive_task_delta(), 0);
    /// ```
    #[must_use]
    pub const fn alive_task_delta(self) -> isize {
        // `cast_signed` rather than `as isize`: a task count is a small
        // non-negative number, and a wrapping cast would be a silent error
        // rather than a surfaced one if that ever stopped being true.
        self.alive_tasks_after.cast_signed() - self.alive_tasks_before.cast_signed()
    }
}

/// Live task count for the current runtime.
///
/// `RuntimeMetrics::num_alive_tasks` is gated only on 64-bit atomics, so it is
/// available without the `rt-metrics` feature or `tokio_unstable`.
fn alive_tasks() -> usize {
    tokio::runtime::Handle::current()
        .metrics()
        .num_alive_tasks()
}

/// Build the pool configuration for a workload.
fn pool_config(workload: PoolWorkload) -> wireframe::client::ClientPoolConfig {
    let config = wireframe::client::ClientPoolConfig::default().pool_size(workload.pool_size);
    match workload.mode.policy() {
        Some(policy) => config.fairness_policy(policy),
        None => config,
    }
}

/// Acquire and immediately release `rounds` leases, counting the grants.
async fn acquire_round(
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

/// Grant count and task-churn counters for one acquisition phase.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct PhaseObservation {
    /// Leases granted during the phase.
    granted: u64,
    /// Scheduler worker tasks spawned during the phase.
    spawns: u64,
    /// Live tasks immediately before the phase.
    alive_before: usize,
    /// Live tasks immediately after the phase.
    alive_after: usize,
}

/// Run one iteration, bracketing only the acquisition phase.
///
/// Pool construction and teardown happen *outside* the bracketed region. That
/// placement is deliberate and was forced by measurement: `build_preamble_pool`
/// connects eagerly, so at pool sizes 1, 4, and 16 the construction phase alone
/// leaves exactly that many extra tasks alive. Sampling the task gauge across
/// construction would therefore report `pool_size` as though it were
/// acquisition churn, which is the opposite of what this scope measures.
async fn pool_once(workload: PoolWorkload) -> Result<PhaseObservation, String> {
    let (_server, pool, _preamble) = build_preamble_pool(pool_config(workload))
        .await
        .map_err(|err| format!("failed to build pooled client: {err}"))?;

    let spawns_before = pool_scheduler_task_spawns();
    let alive_before = alive_tasks();

    let granted = if workload.mode.is_contended() {
        contended_round(workload, &pool).await?
    } else {
        let mut handle = pool.handle();
        acquire_round(&mut handle, workload.acquisitions)
            .await
            .map_err(|err| format!("uncontended acquire failed: {err}"))?
    };

    // A worker spawned by the final release observes an empty queue and exits
    // on its first poll. Yielding here lets that happen before the gauge is
    // read, so `alive_after` reflects settled state rather than a task that is
    // already on its way out.
    tokio::task::yield_now().await;
    let alive_after = alive_tasks();
    let spawns = pool_scheduler_task_spawns().saturating_sub(spawns_before);

    pool.close().await;

    Ok(PhaseObservation {
        granted,
        spawns,
        alive_before,
        alive_after,
    })
}

/// Race every contender for pool capacity behind a shared start barrier.
///
/// The barrier matters: without it the first caller can complete every round
/// before the others are polled, which would make a "contended" label describe
/// an uncontended run. Contenders are given more rounds than the pool has slots
/// so that demand genuinely exceeds capacity once they are all running.
async fn contended_round(
    workload: PoolWorkload,
    pool: &wireframe::client::WireframeClientPool<
        wireframe::serializer::BincodeSerializer,
        wireframe::test_helpers::pool_client::ClientHello,
        (),
    >,
) -> Result<u64, String> {
    let contenders = workload.mode.contenders(workload.contenders);
    let barrier = Arc::new(Barrier::new(contenders));
    let granted = Arc::new(AtomicUsize::new(0));
    let failures = Arc::new(Mutex::new(Vec::<String>::new()));

    let mut tasks = Vec::with_capacity(contenders);
    for _ in 0..contenders {
        let mut handle = pool.handle();
        let barrier = Arc::clone(&barrier);
        let granted = Arc::clone(&granted);
        let failures = Arc::clone(&failures);
        tasks.push(tokio::spawn(async move {
            barrier.wait().await;
            match acquire_round(&mut handle, CONTENDED_ROUNDS).await {
                Ok(count) => {
                    granted.fetch_add(
                        usize::try_from(count).unwrap_or(usize::MAX),
                        Ordering::SeqCst,
                    );
                }
                Err(err) => {
                    let mut guard = match failures.lock() {
                        Ok(guard) => guard,
                        Err(poisoned) => poisoned.into_inner(),
                    };
                    guard.push(err.to_string());
                }
            }
        }));
    }

    for task in tasks {
        task.await
            .map_err(|err| format!("contended contender panicked: {err}"))?;
    }

    let errors = {
        let guard = match failures.lock() {
            Ok(guard) => guard,
            Err(poisoned) => poisoned.into_inner(),
        };
        guard.clone()
    };
    if let Some(first) = errors.first() {
        return Err(format!(
            "{} of {contenders} contenders failed; first: {first}",
            errors.len()
        ));
    }

    Ok(u64::try_from(granted.load(Ordering::SeqCst)).unwrap_or(u64::MAX))
}

/// Attempt `iterations` pool workloads, reporting the last iteration's counts.
///
/// # Errors
///
/// Returns a description of the invalid workload, pool-construction failure, or
/// acquisition failure. A contended workload whose contender count does not
/// exceed its pool size is rejected, because it could not enter the scheduler's
/// slow path and would report an uncontended finding under a contended label.
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
        return Ok(PoolMeasurement {
            acquisitions: 0,
            elapsed: Duration::ZERO,
            scheduler_spawns: 0,
            alive_tasks_before: alive_tasks(),
            alive_tasks_after: alive_tasks(),
        });
    }

    let started = Instant::now();
    let mut last = PhaseObservation {
        granted: 0,
        spawns: 0,
        alive_before: 0,
        alive_after: 0,
    };
    for _ in 0..iterations {
        last = pool_once(workload).await?;
    }
    let elapsed = started.elapsed();

    // The counters are per-iteration totals because each iteration builds and
    // closes its own pool; summing them would count construction churn that the
    // bracket deliberately excludes. The last iteration's figures are the ones
    // the benchmark label and the assertions quote.
    Ok(PoolMeasurement {
        acquisitions: black_box(last.granted),
        elapsed,
        scheduler_spawns: last.spawns,
        alive_tasks_before: last.alive_before,
        alive_tasks_after: last.alive_after,
    })
}

#[cfg(test)]
mod tests {
    //! Unit coverage for pool workload labels and rejection rules.

    use super::{AcquisitionMode, PoolWorkload, measure_pool_acquisition};

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
    /// assumption. It is recorded as a test rather than a comment so the
    /// corrected contract is pinned: if a later refactor removes the per-release
    /// notify, this fails and the baseline table must be revisited.
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
        let measurement = measure_pool_acquisition(
            PoolWorkload {
                pool_size: 4,
                mode,
                acquisitions: 8,
                contenders: 16,
            },
            1,
        )
        .await
        .expect("contended measurement failed");

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

        // The pool is closed before the measurement returns, so the settled
        // delta cannot still show the eagerly-built connections.
        assert!(
            measurement.alive_task_delta() <= 0,
            "a closed pool must not leave construction tasks behind: delta {} on a \
             {pool_size}-slot pool (before {}, after {})",
            measurement.alive_task_delta(),
            measurement.alive_tasks_before,
            measurement.alive_tasks_after
        );
    }
}
