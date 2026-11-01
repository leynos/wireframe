//! The actor-drain scenario as data.
//!
//! The parameters a baseline must state, the frame counts each cancellation
//! regime must produce, and the result a measurement reports. All of it is
//! derived from the workload's own shape, so a caller can assert the expected
//! behaviour without running the actor, and a scenario can be pointed at an old
//! and a new implementation unchanged.

use std::time::Duration;

/// Frame count used by the benchmark's default actor workload.
pub const DEFAULT_FRAME_COUNT: usize = 1024;

/// Per-queue capacity used by the benchmark's default actor workload.
///
/// At least half [`DEFAULT_FRAME_COUNT`], so the default workload satisfies the
/// fit-in-queues precondition without relying on back-pressure.
pub const DEFAULT_QUEUE_CAPACITY: usize = 512;

/// How a workload treats the actor's shutdown token.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CancellationMode {
    /// Never cancel; the actor ends when its queues close.
    Disabled,
    /// Cancel before the loop starts, so the actor aborts without draining.
    CancelledBeforeRun,
    /// Cancel from the output hook on the first frame, mid-drain.
    CancelledDuringDrain,
}

impl CancellationMode {
    /// Short identifier used in benchmark labels.
    ///
    /// # Examples
    ///
    /// ```
    /// use wireframe_testing::runtime_ownership_benchmarks::CancellationMode;
    ///
    /// assert_eq!(CancellationMode::Disabled.label(), "cancel_disabled");
    /// ```
    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Self::Disabled => "cancel_disabled",
            Self::CancelledBeforeRun => "cancel_before_run",
            Self::CancelledDuringDrain => "cancel_during_drain",
        }
    }
}

/// One actor-drain scenario.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ActorWorkload {
    /// Total frames preloaded across both queues.
    pub frames: usize,
    /// Capacity of each of the high- and low-priority queues.
    pub queue_capacity: usize,
    /// Cancellation regime to exercise.
    pub cancellation: CancellationMode,
}

impl Default for ActorWorkload {
    fn default() -> Self {
        Self {
            frames: DEFAULT_FRAME_COUNT,
            queue_capacity: DEFAULT_QUEUE_CAPACITY,
            cancellation: CancellationMode::Disabled,
        }
    }
}

impl ActorWorkload {
    /// Frames placed on the high-priority queue.
    ///
    /// The high queue takes the larger half so an odd frame count still fills
    /// both queues, keeping "both priorities" genuinely exercised.
    ///
    /// # Examples
    ///
    /// ```
    /// use wireframe_testing::runtime_ownership_benchmarks::{ActorWorkload, CancellationMode};
    ///
    /// let workload = ActorWorkload {
    ///     frames: 17,
    ///     queue_capacity: 512,
    ///     cancellation: CancellationMode::Disabled,
    /// };
    /// assert_eq!((workload.high_frames(), workload.low_frames()), (9, 8));
    /// ```
    #[must_use]
    pub const fn high_frames(self) -> usize { self.frames.div_ceil(2) }

    /// Frames placed on the low-priority queue.
    #[must_use]
    pub const fn low_frames(self) -> usize { self.frames - self.high_frames() }

    /// Benchmark label embedding every recorded parameter.
    ///
    /// # Examples
    ///
    /// ```
    /// use wireframe_testing::runtime_ownership_benchmarks::{ActorWorkload, CancellationMode};
    ///
    /// let workload = ActorWorkload {
    ///     frames: 256,
    ///     queue_capacity: 512,
    ///     cancellation: CancellationMode::Disabled,
    /// };
    /// assert_eq!(workload.label(), "frames_256_capacity_512_cancel_disabled");
    /// ```
    #[must_use]
    pub fn label(self) -> String {
        format!(
            "frames_{}_capacity_{}_{}",
            self.frames,
            self.queue_capacity,
            self.cancellation.label()
        )
    }

    /// Number of frames this workload must emit.
    ///
    /// The abort path emits nothing; every other regime emits every preloaded
    /// frame, including the one which observes shutdown mid-drain.
    ///
    /// # Examples
    ///
    /// ```
    /// use wireframe_testing::runtime_ownership_benchmarks::{ActorWorkload, CancellationMode};
    ///
    /// let mut workload = ActorWorkload {
    ///     frames: 17,
    ///     queue_capacity: 512,
    ///     cancellation: CancellationMode::Disabled,
    /// };
    /// assert_eq!(workload.expected_frames(), 17);
    /// workload.cancellation = CancellationMode::CancelledBeforeRun;
    /// assert_eq!(workload.expected_frames(), 0);
    /// ```
    #[must_use]
    pub const fn expected_frames(self) -> usize {
        match self.cancellation {
            CancellationMode::CancelledBeforeRun => 0,
            CancellationMode::Disabled | CancellationMode::CancelledDuringDrain => self.frames,
        }
    }

    /// Reject workloads the harness cannot measure as described.
    pub(super) fn validate(self) -> Result<(), String> {
        if self.queue_capacity == 0 {
            return Err("queue_capacity must be at least 1".to_owned());
        }
        let headroom = self
            .queue_capacity
            .checked_mul(2)
            .ok_or_else(|| "queue_capacity is too large to double".to_owned())?;
        if self.frames > headroom {
            return Err(format!(
                "frames {} exceeds total queue capacity {}; preloading would block on \
                 back-pressure and change the measured regime",
                self.frames, headroom
            ));
        }
        Ok(())
    }
}

/// Result of driving the actor for one measurement.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ActorMeasurement {
    /// Workload iterations completed.
    pub operations: u64,
    /// Frames emitted by the final iteration.
    pub frames_emitted: usize,
    /// Wall-clock time across every iteration.
    pub elapsed: Duration,
    /// Whether the output hook observed a cancellation request mid-drain.
    ///
    /// Recorded so that a test can tell "cancellation was requested during the
    /// drain" from "the hook silently never fired", which would otherwise make
    /// [`CancellationMode::CancelledDuringDrain`] indistinguishable from
    /// [`CancellationMode::Disabled`].
    pub cancelled_during_drain: bool,
}
