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
///
/// The four awaited scenarios are separated by two questions, and both must be
/// answered explicitly because neither is implied by the other. Does a consumer
/// drain the queue, and can the queue hold the whole run? A queue that holds
/// every frame cannot make a producer park even with no consumer at all; a queue
/// that cannot hold them will park the producer only if nothing is draining it.
/// Answering both is what makes each scenario's regime a property of the
/// workload rather than of the scheduler's timing.
///
/// | Scenario | Consumer drains | Queue holds run | Waits |
/// | --- | --- | --- | --- |
/// | `UncontendedHigh` / `UncontendedLow` | yes | yes | always 0 |
/// | `DrainedHigh` / `DrainedLow` | yes | no | workload-dependent |
/// | `BackpressureHigh` | only after a park | no | at least `sends - capacity` |
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PushScenario {
    /// Awaiting sends into a draining consumer, in a queue holding the whole run.
    ///
    /// The no-back-pressure control: waiting is impossible, not merely unlikely,
    /// so a zero wait count is a real result rather than a lucky one.
    UncontendedHigh,
    /// The low-priority equivalent of [`PushScenario::UncontendedHigh`].
    UncontendedLow,
    /// Awaiting sends into a draining consumer, in a queue too small for the run.
    ///
    /// The realistic producer/consumer throughput case: the producer may outrun
    /// the consumer and park, so the wait counters are reported rather than
    /// asserted. A non-zero reading here is a finding about the workload, not a
    /// failure of it — which is exactly what the previously-mislabelled
    /// `uncontended_*` scenarios could not express.
    DrainedHigh,
    /// The low-priority equivalent of [`PushScenario::DrainedHigh`].
    DrainedLow,
    /// Awaiting sends that must park, because nothing drains the queue.
    BackpressureHigh,
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
    /// assert_eq!(PushScenario::DrainedLow.label(), "drained_low");
    /// ```
    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Self::UncontendedHigh => "uncontended_high",
            Self::UncontendedLow => "uncontended_low",
            Self::DrainedHigh => "drained_high",
            Self::DrainedLow => "drained_low",
            Self::BackpressureHigh => "backpressure_high",
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
    /// assert!(!PushScenario::BackpressureHigh.is_overflow());
    /// assert!(PushScenario::OverflowDrop.is_overflow());
    /// ```
    #[must_use]
    pub const fn is_overflow(self) -> bool {
        matches!(
            self,
            Self::OverflowReturnError | Self::OverflowDrop | Self::OverflowWarnAndDrop
        )
    }

    /// Whether this scenario measures an awaited send that must wait for capacity.
    ///
    /// # Examples
    ///
    /// ```
    /// use wireframe_testing::runtime_ownership_benchmarks::PushScenario;
    ///
    /// assert!(PushScenario::BackpressureHigh.is_backpressure());
    /// assert!(!PushScenario::UncontendedHigh.is_backpressure());
    /// ```
    #[must_use]
    pub const fn is_backpressure(self) -> bool { matches!(self, Self::BackpressureHigh) }

    /// Whether this scenario measures an awaited send that cannot wait.
    ///
    /// # Examples
    ///
    /// ```
    /// use wireframe_testing::runtime_ownership_benchmarks::PushScenario;
    ///
    /// assert!(PushScenario::UncontendedLow.is_uncontended());
    /// assert!(!PushScenario::DrainedHigh.is_uncontended());
    /// ```
    #[must_use]
    pub const fn is_uncontended(self) -> bool {
        matches!(self, Self::UncontendedHigh | Self::UncontendedLow)
    }

    /// Whether this scenario awaits into a queue a consumer is draining.
    ///
    /// # Examples
    ///
    /// ```
    /// use wireframe_testing::runtime_ownership_benchmarks::PushScenario;
    ///
    /// assert!(PushScenario::DrainedHigh.is_drained());
    /// assert!(!PushScenario::BackpressureHigh.is_drained());
    /// ```
    #[must_use]
    pub const fn is_drained(self) -> bool {
        self.is_uncontended() || matches!(self, Self::DrainedHigh | Self::DrainedLow)
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
    pub const fn policy(self) -> Option<PushPolicy> {
        match self {
            Self::OverflowReturnError => Some(PushPolicy::ReturnErrorIfFull),
            Self::OverflowDrop => Some(PushPolicy::DropIfFull),
            Self::OverflowWarnAndDrop => Some(PushPolicy::WarnAndDropIfFull),
            Self::UncontendedHigh
            | Self::UncontendedLow
            | Self::DrainedHigh
            | Self::DrainedLow
            | Self::BackpressureHigh => None,
        }
    }

    /// The queue lane this scenario drives.
    pub(super) const fn lane(self) -> Lane {
        match self {
            Self::UncontendedLow | Self::DrainedLow => Lane::Low,
            Self::UncontendedHigh
            | Self::DrainedHigh
            | Self::BackpressureHigh
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
            // Drained rather than uncontended: at the default send count the
            // queue cannot hold the run, so the producer may legitimately park
            // and the wait counters are reported instead of asserted to zero.
            // Claiming `UncontendedHigh` here would be rejected by `validate`.
            scenario: PushScenario::DrainedHigh,
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
    ///
    /// The capacity checks are not arithmetic hygiene: they are what makes each
    /// scenario's regime a property of the workload rather than of the
    /// scheduler's mood. A producer sending no more frames than the queue holds
    /// can acquire every slot it needs without any consumer running, so it
    /// *cannot* park; one sending more than a full queue that nothing drains must
    /// park, so it cannot help but. Either guarantee is exactly what the
    /// corresponding scenario's wait-counter assertion needs; a workload that
    /// violates it would be measured under a label its own counters contradict.
    ///
    /// The drained-but-overflowing scenarios are the one pair with no capacity
    /// constraint, and deliberately so: whether a consumer keeps up is a fact
    /// about the running system, which is what those scenarios exist to observe.
    ///
    /// # Errors
    ///
    /// Returns a description of whichever constraint the workload breaks.
    pub(super) fn validate(self) -> Result<(), String> {
        if self.queue_capacity == 0 {
            return Err("queue_capacity must be at least 1".to_owned());
        }
        if self.sends == 0 {
            return Err("sends must be at least 1".to_owned());
        }
        if self.scenario.is_uncontended() && self.sends > self.queue_capacity {
            return Err(format!(
                "scenario {} claims the uncontended regime but sends {} frames into a queue of \
                 capacity {}, so the producer can outrun any consumer and park; size the queue to \
                 hold the whole run, use {} to observe a consumer's throughput, or {} to measure \
                 the parked path",
                self.scenario.label(),
                self.sends,
                self.queue_capacity,
                PushScenario::DrainedHigh.label(),
                PushScenario::BackpressureHigh.label()
            ));
        }
        if self.scenario.is_backpressure() && self.sends <= self.queue_capacity {
            return Err(format!(
                "scenario {} claims to measure awaited back-pressure but sends {} frames into a \
                 queue of capacity {}, so no send can ever park; send more than the queue holds",
                self.scenario.label(),
                self.sends,
                self.queue_capacity
            ));
        }
        Ok(())
    }
}

/// Result of measuring one push workload.
///
/// The counts are deliberately separate because a non-blocking send has two
/// distinct notions of success. `accepted` is what the push API reported; under
/// a drop policy an accepted frame was *not* queued, it was routed to the
/// dead-letter queue, so accepted frames and dead-lettered frames are the same
/// events rather than complementary ones. Comparing the counters is how a
/// caller tells the regimes apart:
///
/// | Scenario | `accepted` | `dead_lettered` | `reported_queue_full` |
/// | --- | --- | --- | --- |
/// | Uncontended | `sends` | 0 | `false` |
/// | `Backpressure` | `sends` | 0 | `false` |
/// | `OverflowReturnError` | 0 | 0 | `true` |
/// | `OverflowDrop` / `OverflowWarnAndDrop` | `sends` | `sends` | `false` |
///
/// [`PushError::QueueFull`](wireframe::push::PushError::QueueFull) is the
/// rejection the last column reports. The uncontended and back-pressure rows
/// are identical in those three columns, which is exactly why
/// [`PushMeasurement::awaited_capacity_waits`] exists: it is the only field
/// that tells "awaited a send without waiting" from "awaited a send that had to
/// park".
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PushMeasurement {
    /// Total sends attempted across every iteration.
    pub sends: u64,
    /// Sends the push API reported as accepted.
    pub accepted: usize,
    /// Frames a consumer received; zero only for the overflow scenarios, whose
    /// queues are deliberately never drained.
    pub drained: usize,
    /// Frames the dead-letter queue received.
    pub dead_lettered: usize,
    /// Whether any send reported
    /// [`PushError::QueueFull`](wireframe::push::PushError::QueueFull).
    ///
    /// Only a non-blocking send can report this, so it is `false` for every
    /// awaited scenario however full their queues were. See
    /// [`PushMeasurement::awaited_capacity_waits`].
    pub reported_queue_full: bool,
    /// Awaited sends that found their queue full and parked for capacity.
    ///
    /// The observable that [`PushMeasurement::reported_queue_full`] cannot
    /// supply for an awaited send: a park is a wait, not a failure, so it leaves
    /// no error behind. An uncontended scenario must report zero here, and a
    /// back-pressure scenario must report non-zero, which is what makes the two
    /// regimes evidence rather than assertion.
    ///
    /// The count is of sends whose first poll returned `Pending`, not of sends
    /// that merely found the queue full: a caller that releases capacity before
    /// the send is polled has not waited, and this does not claim it did.
    pub awaited_capacity_waits: u64,
    /// Awaited sends that reached the channel and probed it for capacity.
    ///
    /// The denominator for the count above. Zero waits beside zero probes means
    /// nothing was measured, not that nothing waited, so a no-back-pressure
    /// claim needs this non-zero as well.
    pub capacity_probes: u64,
    /// Wall-clock time across every iteration.
    pub elapsed: Duration,
}
