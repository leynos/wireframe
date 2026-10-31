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

/// Acquire/release rounds each contender performs in a contended burst.
///
/// This is the parameter that sets a contended burst's length, because a burst
/// grants `contenders × this` leases. It lives beside the workload data rather
/// than inside the measurement code because it is a recorded parameter: it
/// belongs in the benchmark label, so a burst whose length changed cannot be
/// compared against an older baseline unnoticed.
pub const CONTENDED_ROUNDS_PER_CONTENDER: usize = 8;

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
    pub(super) const fn configure(
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
    ///
    /// Read only when [`AcquisitionMode::is_contended`] is false. A contended
    /// burst is driven by [`CONTENDED_ROUNDS_PER_CONTENDER`] instead, so this
    /// field is ignored in that mode and is deliberately absent from the
    /// contended label rather than reported as though it had been honoured.
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
    /// let uncontended = PoolWorkload {
    ///     pool_size: 4,
    ///     mode: AcquisitionMode::Uncontended,
    ///     acquisitions: 256,
    ///     contenders: 16,
    /// };
    /// assert_eq!(
    ///     uncontended.label(),
    ///     "uncontended_pool_4_acquisitions_256_contenders_1"
    /// );
    ///
    /// let contended = PoolWorkload {
    ///     mode: AcquisitionMode::ContendedFifo,
    ///     ..uncontended
    /// };
    /// assert_eq!(
    ///     contended.label(),
    ///     "contended_fifo_pool_4_contenders_16_rounds_8"
    /// );
    /// ```
    ///
    /// The two regimes name different parameters, because they are driven by
    /// different ones. An uncontended workload is one caller looping
    /// [`PoolWorkload::acquisitions`] times, so that count is what the label
    /// states. A contended workload is every contender running
    /// [`CONTENDED_ROUNDS_PER_CONTENDER`] rounds, and the burst grants
    /// `contenders × rounds` leases, so labelling it `acquisitions_256` would
    /// advertise a parameter the contended path never reads.
    #[must_use]
    pub fn label(self) -> String {
        let mode = self.mode.label();
        let pool_size = self.pool_size;
        let contenders = self.mode.contenders(self.contenders);
        if self.mode.is_contended() {
            let rounds = CONTENDED_ROUNDS_PER_CONTENDER;
            format!("{mode}_pool_{pool_size}_contenders_{contenders}_rounds_{rounds}")
        } else {
            let acquisitions = self.acquisitions;
            format!("{mode}_pool_{pool_size}_acquisitions_{acquisitions}_contenders_{contenders}")
        }
    }

    /// Reject workloads whose regime cannot produce the documented outcome.
    pub(super) fn validate(self) -> Result<(), String> {
        if self.pool_size == 0 {
            return Err("pool_size must be at least 1".to_owned());
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

/// FIFO contended workloads for every pool size used by runtime baselines.
///
/// The default of sixteen contenders exceeds pool sizes one and four. A
/// 16-slot pool needs one additional contender for the harness to observe a
/// capacity wait, so its label records seventeen.
///
/// # Examples
///
/// ```
/// use wireframe_testing::runtime_ownership_benchmarks::contended_fifo_pool_workloads;
///
/// let workloads = contended_fifo_pool_workloads();
/// assert_eq!(workloads[0].pool_size, 1);
/// assert_eq!(workloads[2].contenders, 17);
/// ```
#[must_use]
pub fn contended_fifo_pool_workloads() -> [PoolWorkload; POOL_SIZES.len()] {
    POOL_SIZES.map(|pool_size| PoolWorkload {
        pool_size,
        mode: AcquisitionMode::ContendedFifo,
        contenders: DEFAULT_CONTENDERS.max(pool_size.saturating_add(1)),
        ..PoolWorkload::default()
    })
}

/// Result of measuring one pool workload.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PoolMeasurement {
    /// Acquisitions completed (granted leases) in the final iteration.
    pub acquisitions: u64,
    /// Wall-clock time across every acquisition phase.
    ///
    /// Excludes pool construction and warm-up. Those are real costs of the
    /// scope but not of the acquire/release path this field exists to report;
    /// they appear in [`PoolMeasurement::construction`] instead.
    pub elapsed: Duration,
    /// Wall-clock time spent building and warming the measured pool.
    ///
    /// Reported rather than folded into [`PoolMeasurement::elapsed`] because
    /// construction is dominated by `pool_size` eager loopback connections. A
    /// single figure covering both would be mostly a measure of how many sockets
    /// the pool holds, which is a property of the configuration rather than of
    /// the acquisition path.
    pub construction: Duration,
    /// Scheduler worker tasks spawned by the measured pool.
    ///
    /// Scoped to the pool under measurement, not to the process, so a
    /// concurrent unrelated pool cannot contribute to this number.
    pub scheduler_spawns: u64,
    /// Acquisitions that had to wait for capacity on the measured pool.
    ///
    /// The direct evidence that the pool was genuinely exhausted. A contended
    /// scenario asserts this is non-zero, because contenders outnumbering slots
    /// only makes exhaustion possible — this counts it happening.
    pub capacity_waits: u64,
    /// Live tasks immediately before the first acquisition phase.
    ///
    /// Sampled after the pool is built and warmed, so eager bb8 connection tasks
    /// are already running and are excluded from the delta below.
    pub alive_tasks_before: usize,
    /// Live runtime tasks observed after the final acquisition phase settled.
    pub alive_tasks_after: usize,
}

impl PoolMeasurement {
    /// Change in live task count across the acquisition phases.
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
    ///     construction: Duration::ZERO,
    ///     scheduler_spawns: 0,
    ///     capacity_waits: 0,
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

    /// Whether the measured pool was observed to run out of capacity.
    ///
    /// # Examples
    ///
    /// ```
    /// use std::time::Duration;
    ///
    /// use wireframe_testing::runtime_ownership_benchmarks::PoolMeasurement;
    ///
    /// let uncontended = PoolMeasurement {
    ///     acquisitions: 16,
    ///     elapsed: Duration::ZERO,
    ///     construction: Duration::ZERO,
    ///     scheduler_spawns: 1,
    ///     capacity_waits: 0,
    ///     alive_tasks_before: 2,
    ///     alive_tasks_after: 2,
    /// };
    /// assert!(!uncontended.contended());
    /// ```
    #[must_use]
    pub const fn contended(self) -> bool { self.capacity_waits > 0 }
}
