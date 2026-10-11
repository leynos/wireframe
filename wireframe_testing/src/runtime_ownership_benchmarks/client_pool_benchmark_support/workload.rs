//! The pool-acquisition scenarios as data.
//!
//! The parameters a baseline must state, the fairness policy each mode
//! configures, and the counters a measurement reports. A caller can classify a
//! workload — contended or not, which policy — without building a pool.

use std::time::Duration;

use wireframe::client::PoolFairnessPolicy;

/// Pool sizes the issue asks to cover.
pub const POOL_SIZES: [usize; 3] = [1, 4, 16];

/// Acquisitions used by the benchmark's default uncontended workload.
pub const DEFAULT_ACQUISITIONS: usize = 256;

/// Contenders used by the benchmark's default contended workload.
pub const DEFAULT_CONTENDERS: usize = 16;

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
    pub(super) const fn contenders(self, requested: usize) -> usize {
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

    /// Build the pool configuration this mode requires.
    pub(super) fn configure(
        self,
        config: wireframe::client::ClientPoolConfig,
    ) -> wireframe::client::ClientPoolConfig {
        match self.policy() {
            Some(policy) => config.fairness_policy(policy),
            None => config,
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
    pub(super) fn validate(self) -> Result<(), String> {
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
