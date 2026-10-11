//! The push-send scenarios as data.
//!
//! The parameters a baseline must state, the policy each scenario branches on,
//! and the counters a measurement reports. A caller can classify a scenario —
//! lane, policy, overflow or not — without driving the push path, which is what
//! lets the test suite assert the regimes are wired to the right branches.

use std::time::Duration;

use wireframe::push::PushPolicy;

/// Queue capacity used by the benchmark's default push workload.
pub const DEFAULT_QUEUE_CAPACITY: usize = 64;

/// Send count used by the benchmark's default uncontended workload.
pub const DEFAULT_SENDS: usize = 1024;

/// Priority lane a scenario targets.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Lane {
    /// The high-priority queue.
    High,
    /// The low-priority queue.
    Low,
}

impl Lane {
    /// The matching priority value for non-blocking sends.
    pub(super) const fn priority(self) -> wireframe::push::PushPriority {
        match self {
            Self::High => wireframe::push::PushPriority::High,
            Self::Low => wireframe::push::PushPriority::Low,
        }
    }
}

/// One push-handle scenario.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PushScenario {
    /// Awaiting sends into a draining high-priority queue.
    UncontendedHigh,
    /// Awaiting sends into a draining low-priority queue.
    UncontendedLow,
    /// Non-blocking send against a full queue under `ReturnErrorIfFull`.
    OverflowReturnError,
    /// Non-blocking send against a full queue under `DropIfFull`.
    OverflowDrop,
    /// Non-blocking send against a full queue under `WarnAndDropIfFull`.
    OverflowWarnAndDrop,
}

impl PushScenario {
    /// Short identifier used in benchmark labels.
    ///
    /// # Examples
    ///
    /// ```
    /// use wireframe_testing::runtime_ownership_benchmarks::PushScenario;
    ///
    /// assert_eq!(PushScenario::UncontendedHigh.label(), "uncontended_high");
    /// ```
    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Self::UncontendedHigh => "uncontended_high",
            Self::UncontendedLow => "uncontended_low",
            Self::OverflowReturnError => "overflow_return_error",
            Self::OverflowDrop => "overflow_drop",
            Self::OverflowWarnAndDrop => "overflow_warn_and_drop",
        }
    }

    /// Whether this scenario measures a non-blocking overflow send.
    ///
    /// # Examples
    ///
    /// ```
    /// use wireframe_testing::runtime_ownership_benchmarks::PushScenario;
    ///
    /// assert!(!PushScenario::UncontendedHigh.is_overflow());
    /// assert!(PushScenario::OverflowDrop.is_overflow());
    /// ```
    #[must_use]
    pub const fn is_overflow(self) -> bool {
        !matches!(self, Self::UncontendedHigh | Self::UncontendedLow)
    }

    /// Queue lane this scenario targets, as a label-safe string.
    ///
    /// # Examples
    ///
    /// ```
    /// use wireframe_testing::runtime_ownership_benchmarks::PushScenario;
    ///
    /// assert_eq!(PushScenario::UncontendedLow.priority(), "low");
    /// ```
    #[must_use]
    pub const fn priority(self) -> &'static str {
        match self.lane() {
            Lane::High => "high",
            Lane::Low => "low",
        }
    }

    /// The policy used for an overflow send, or `None` when not applicable.
    ///
    /// # Examples
    ///
    /// ```
    /// use wireframe_testing::runtime_ownership_benchmarks::PushScenario;
    ///
    /// assert_eq!(PushScenario::UncontendedHigh.policy(), None);
    /// assert!(PushScenario::OverflowDrop.policy().is_some());
    /// ```
    #[must_use]
    pub fn policy(self) -> Option<PushPolicy> {
        match self {
            Self::OverflowReturnError => Some(PushPolicy::ReturnErrorIfFull),
            Self::OverflowDrop => Some(PushPolicy::DropIfFull),
            Self::OverflowWarnAndDrop => Some(PushPolicy::WarnAndDropIfFull),
            Self::UncontendedHigh | Self::UncontendedLow => None,
        }
    }

    /// The queue lane this scenario drives.
    pub(super) const fn lane(self) -> Lane {
        match self {
            Self::UncontendedLow => Lane::Low,
            Self::UncontendedHigh
            | Self::OverflowReturnError
            | Self::OverflowDrop
            | Self::OverflowWarnAndDrop => Lane::High,
        }
    }

    /// The overflow send policy, rejecting the uncontended scenarios.
    pub(super) fn overflow_policy(self) -> Result<PushPolicy, String> {
        self.policy()
            .ok_or_else(|| format!("scenario {} is not an overflow scenario", self.label()))
    }
}

/// One push-handle workload.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PushWorkload {
    /// Scenario to measure.
    pub scenario: PushScenario,
    /// Capacity of the queue the scenario targets.
    pub queue_capacity: usize,
    /// Sends attempted per iteration.
    pub sends: usize,
}

impl Default for PushWorkload {
    fn default() -> Self {
        Self {
            scenario: PushScenario::UncontendedHigh,
            queue_capacity: DEFAULT_QUEUE_CAPACITY,
            sends: DEFAULT_SENDS,
        }
    }
}

impl PushWorkload {
    /// Benchmark label embedding every recorded parameter.
    ///
    /// # Examples
    ///
    /// ```
    /// use wireframe_testing::runtime_ownership_benchmarks::{PushScenario, PushWorkload};
    ///
    /// let workload = PushWorkload {
    ///     scenario: PushScenario::UncontendedHigh,
    ///     queue_capacity: 64,
    ///     sends: 1024,
    /// };
    /// assert_eq!(workload.label(), "uncontended_high_capacity_64_sends_1024");
    /// ```
    ///
    /// The lane is deliberately absent: every scenario name already states
    /// whether it targets the high- or low-priority queue, so appending it
    /// would spell `high` twice in the overflow cases, which all send high.
    /// [`PushScenario::priority`] still drives the send itself.
    #[must_use]
    pub fn label(self) -> String {
        format!(
            "{}_capacity_{}_sends_{}",
            self.scenario.label(),
            self.queue_capacity,
            self.sends
        )
    }

    /// Reject workloads the harness cannot measure as described.
    pub(super) fn validate(self) -> Result<(), String> {
        if self.queue_capacity == 0 {
            return Err("queue_capacity must be at least 1".to_string());
        }
        if self.sends == 0 {
            return Err("sends must be at least 1".to_string());
        }
        Ok(())
    }
}

/// Result of measuring one push workload.
///
/// The three counts are deliberately separate because a non-blocking send has
/// two distinct notions of success. `accepted` is what the push API reported;
/// under a drop policy an accepted frame was *not* queued, it was routed to the
/// dead-letter queue, so accepted frames and dead-lettered frames are the same
/// events rather than complementary ones. Comparing the counters is how a
/// caller tells the regimes apart:
///
/// | Scenario | `accepted` | `dead_lettered` | `reported_queue_full` |
/// | --- | --- | --- | --- |
/// | Uncontended | `sends` | 0 | `false` |
/// | `OverflowReturnError` | 0 | 0 | `true` |
/// | `OverflowDrop` / `OverflowWarnAndDrop` | `sends` | `sends` | `false` |
///
/// [`PushError::QueueFull`](wireframe::push::PushError::QueueFull) is the
/// rejection the last column reports.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PushMeasurement {
    /// Total sends attempted across every iteration.
    pub sends: u64,
    /// Sends the push API reported as accepted.
    pub accepted: usize,
    /// Frames a consumer received; non-zero only for the uncontended scenarios.
    pub drained: usize,
    /// Frames the dead-letter queue received.
    pub dead_lettered: usize,
    /// Whether any send reported
    /// [`PushError::QueueFull`](wireframe::push::PushError::QueueFull).
    pub reported_queue_full: bool,
    /// Wall-clock time across every iteration.
    pub elapsed: Duration,
}
