//! Workloads and measurement for the push handle's send path.
//!
//! The push handle is the producer-facing half of a connection: application
//! code, background workers, and protocol hooks all enqueue frames through it.
//! Two distinct costs live here, and the issue asks for them separately:
//!
//! - the **uncontended** cost of awaiting a send into a queue a consumer is draining, split by
//!   priority because the actor polls the high-priority queue first; and
//! - the **overflow** cost of a non-blocking send against a full queue, which branches on
//!   [`PushPolicy`] and never awaits capacity.
//!
//! Rate limiting is disabled in every scenario. The default limiter is a leaky
//! bucket capped at 100 pushes per second, so leaving it enabled would make the
//! measured cost almost entirely `leaky_bucket` bookkeeping and sleeping rather
//! than anything about queue ownership.
//!
//! # Drop policies
//!
//! [`PushScenario::OverflowReturnError`] reports [`PushError::QueueFull`].
//! [`PushScenario::OverflowDrop`] and [`PushScenario::OverflowWarnAndDrop`]
//! report success and route the frame to the dead-letter queue instead.
//!
//! The two outcomes are not equally observable. Dropped frames are *routed* to
//! the DLQ, so a receiver on that channel sees each one; the handle's own
//! `dlq_drops` counter is `pub(crate)` and increments only when the DLQ itself
//! is full or closed, so it is neither reachable nor the right number from
//! outside the crate. [`measure_push`] therefore reports
//! [`PushMeasurement::dead_lettered`], drained from the DLQ receiver, which is
//! the black-box observation the issue's method asks for.

use std::{
    hint::black_box,
    time::{Duration, Instant},
};

use tokio::sync::mpsc;
use wireframe::push::{PushError, PushHandle, PushPolicy, PushPriority, PushQueues};

/// Queue capacity used by the benchmark's default push workload.
pub const DEFAULT_QUEUE_CAPACITY: usize = 64;

/// Send count used by the benchmark's default uncontended workload.
pub const DEFAULT_SENDS: usize = 1024;

/// Fallback frame value, used only if the modulo ever exceeds `u8` range.
///
/// Frames are plain `u8` values here: the push path is queue bookkeeping and
/// channel hand-off, so a payload buffer would add allocation noise to a
/// measurement that is not about payload handling. The actor and startup
/// scenarios, which do carry payloads, use [`Envelope`](wireframe::app::Envelope).
const FALLBACK_FRAME: u8 = 0xcd;

/// Priority lane a scenario targets.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Lane {
    /// The high-priority queue.
    High,
    /// The low-priority queue.
    Low,
}

impl Lane {
    /// The matching priority value for non-blocking sends.
    const fn priority(self) -> PushPriority {
        match self {
            Self::High => PushPriority::High,
            Self::Low => PushPriority::Low,
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

    /// Queue lane this scenario targets.
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
    const fn lane(self) -> Lane {
        match self {
            Self::UncontendedLow => Lane::Low,
            Self::UncontendedHigh
            | Self::OverflowReturnError
            | Self::OverflowDrop
            | Self::OverflowWarnAndDrop => Lane::High,
        }
    }

    /// The overflow send policy, rejecting the uncontended scenarios.
    fn overflow_policy(self) -> Result<PushPolicy, String> {
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
    /// assert_eq!(
    ///     workload.label(),
    ///     "uncontended_high_high_capacity_64_sends_1024"
    /// );
    /// ```
    #[must_use]
    pub fn label(self) -> String {
        format!(
            "{}_{}_capacity_{}_sends_{}",
            self.scenario.label(),
            self.scenario.priority(),
            self.queue_capacity,
            self.sends
        )
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
    /// Whether any send reported [`PushError::QueueFull`].
    pub reported_queue_full: bool,
    /// Wall-clock time across every iteration.
    pub elapsed: Duration,
}

/// Queues, producer handle, and dead-letter receiver for one measurement.
type PushFixture = (PushQueues<u8>, PushHandle<u8>, mpsc::Receiver<u8>);

/// Build queues sized for `workload` with rate limiting disabled and a DLQ.
///
/// Returns the queues, the producer handle, and the dead-letter receiver. The
/// DLQ is always attached so that a drop policy's routing is observable rather
/// than silently discarded.
fn build_queues(workload: PushWorkload) -> Result<PushFixture, String> {
    let (dlq_tx, dlq_rx) = mpsc::channel(workload.sends.max(1));
    let (queues, handle) = PushQueues::<u8>::builder()
        .high_capacity(workload.queue_capacity)
        .low_capacity(workload.queue_capacity)
        .unlimited()
        .dlq(Some(dlq_tx))
        .build()
        .map_err(|err| format!("failed to build push queues: {err}"))?;
    Ok((queues, handle, dlq_rx))
}

/// One iteration's outcome.
#[derive(Clone, Copy, Debug, Default)]
struct SendOutcome {
    /// Sends the push API reported as accepted.
    accepted: usize,
    /// Frames a consumer received.
    drained: usize,
    /// Frames observed on the dead-letter queue.
    dead_lettered: usize,
    /// Whether any send reported a full queue.
    reported_queue_full: bool,
}

/// Frame value for one send, kept distinguishable for debug builds.
///
/// A mask rather than `%`: `clippy::integer-division-remainder-used` is denied
/// workspace-wide, and the only property the frames need is that consecutive
/// sends differ, which a power-of-two wrap gives without a division.
fn frame(index: usize) -> u8 { u8::try_from(index & 0xff).unwrap_or(FALLBACK_FRAME) }

/// Run the uncontended scenario against a draining consumer.
async fn send_uncontended(workload: PushWorkload) -> Result<SendOutcome, String> {
    let lane = workload.scenario.lane();
    let (queues, handle, _dlq_rx) = build_queues(workload)?;
    let mut draining = queues;

    // A consumer must be running, otherwise the queue fills and the awaited
    // send measures back-pressure instead of the send path.
    let consumer = tokio::spawn(async move {
        let mut received = 0usize;
        while draining.recv().await.is_some() {
            received += 1;
        }
        received
    });

    for index in 0..workload.sends {
        let frame = frame(index);
        let sent = match lane {
            Lane::High => handle.push_high_priority(frame).await,
            Lane::Low => handle.push_low_priority(frame).await,
        };
        if matches!(sent, Err(PushError::Closed)) {
            return Err("uncontended consumer closed the queue early".to_string());
        }
    }

    // Drop the handle so the queue closes and the consumer finishes; otherwise
    // the join would wait for a producer that is still alive.
    drop(handle);
    let received = consumer
        .await
        .map_err(|err| format!("draining consumer failed: {err}"))?;

    Ok(SendOutcome {
        // An awaited send parks rather than failing when the queue is full, so
        // every send that did not observe a closed queue was accepted. The
        // consumer's count is reported separately: for this scenario the two
        // should agree, and a disagreement means frames were lost.
        accepted: workload.sends,
        drained: received,
        dead_lettered: 0,
        reported_queue_full: false,
    })
}

/// Fill the target queue to capacity, returning how many frames fit.
async fn fill_to_capacity(
    workload: PushWorkload,
    handle: &PushHandle<u8>,
) -> Result<usize, String> {
    let lane = workload.scenario.lane();
    let mut filled = 0usize;
    for index in 0..workload.queue_capacity {
        let frame = frame(index);
        let sent = match lane {
            Lane::High => handle.push_high_priority(frame).await,
            Lane::Low => handle.push_low_priority(frame).await,
        };
        match sent {
            Ok(()) => filled += 1,
            Err(PushError::Closed) => {
                return Err("queue closed while filling to capacity".to_string());
            }
            // `QueueFull` is the expected stop; `PushError` is non-exhaustive,
            // so any future variant also stops here rather than being mistaken
            // for a successful fill. Stopping is safe because the caller checks
            // that the queue reached capacity.
            Err(_) => break,
        }
    }
    Ok(filled)
}

/// Run an overflow scenario against a full queue.
async fn send_overflow(workload: PushWorkload) -> Result<SendOutcome, String> {
    let policy = workload.scenario.overflow_policy()?;
    let (queues, handle, mut dlq_rx) = build_queues(workload)?;
    // The queues are deliberately not drained: overflow is the scenario.
    let filled = fill_to_capacity(workload, &handle).await?;
    if filled < workload.queue_capacity {
        return Err(format!(
            "only {filled} of {} frames fit, so the queue never reached capacity",
            workload.queue_capacity
        ));
    }

    let priority = workload.scenario.lane().priority();
    let mut outcome = SendOutcome::default();
    for index in 0..workload.sends {
        match handle.try_push(frame(index), priority, policy) {
            // A drop policy routes the frame to the DLQ *and* reports success,
            // so an `Ok` here means "the send path completed", not "the frame
            // is queued". Which of the two happened is told apart below by the
            // DLQ count, not by this return value.
            Ok(()) => outcome.accepted += 1,
            Err(PushError::QueueFull) => outcome.reported_queue_full = true,
            Err(PushError::Closed) => {
                return Err("queue closed during overflow sends".to_string());
            }
            // Non-exhaustive enum: an unrecognized error is reported as a
            // diagnostic rather than being silently counted as a drop.
            Err(err) => return Err(format!("overflow send failed: {err}")),
        }
    }

    // Drain the DLQ without awaiting: routing is synchronous, so every
    // dead-lettered frame is already buffered. The preload frames are not
    // counted here — only what the overflow sends routed.
    while dlq_rx.try_recv().is_ok() {
        outcome.dead_lettered += 1;
    }
    drop(queues);
    Ok(outcome)
}

/// Attempt `iterations` of one push workload, reporting the last iteration.
///
/// # Errors
///
/// Returns a description of the invalid workload, queue-construction failure,
/// consumer failure, or a queue that never reached capacity. An overflow
/// scenario whose queue cannot fill would measure the empty-queue path while
/// claiming to measure overflow, so it is rejected instead.
///
/// # Examples
///
/// ```rust,no_run
/// use wireframe_testing::runtime_ownership_benchmarks::{
///     PushScenario,
///     PushWorkload,
///     measure_push,
/// };
///
/// # async fn example() -> Result<(), String> {
/// let measurement = measure_push(
///     PushWorkload {
///         scenario: PushScenario::UncontendedHigh,
///         queue_capacity: 64,
///         sends: 16,
///     },
///     1,
/// )
/// .await?;
/// assert_eq!(measurement.accepted, 16);
/// assert_eq!(measurement.drained, 16);
/// # Ok(())
/// # }
/// ```
pub async fn measure_push(
    workload: PushWorkload,
    iterations: u64,
) -> Result<PushMeasurement, String> {
    if workload.queue_capacity == 0 {
        return Err("queue_capacity must be at least 1".to_string());
    }
    if workload.sends == 0 {
        return Err("sends must be at least 1".to_string());
    }
    if iterations == 0 {
        return Ok(PushMeasurement {
            sends: 0,
            accepted: 0,
            drained: 0,
            dead_lettered: 0,
            reported_queue_full: false,
            elapsed: Duration::ZERO,
        });
    }

    let started = Instant::now();
    let mut outcome = SendOutcome::default();
    for _ in 0..iterations {
        outcome = if workload.scenario.is_overflow() {
            send_overflow(workload).await?
        } else {
            send_uncontended(workload).await?
        };
    }
    let elapsed = started.elapsed();

    Ok(PushMeasurement {
        sends: workload
            .sends
            .saturating_mul(usize::try_from(iterations).unwrap_or(usize::MAX))
            .try_into()
            .unwrap_or(u64::MAX),
        accepted: black_box(outcome.accepted),
        drained: black_box(outcome.drained),
        dead_lettered: black_box(outcome.dead_lettered),
        reported_queue_full: outcome.reported_queue_full,
        elapsed,
    })
}

#[cfg(test)]
mod tests {
    //! Unit coverage for scenario classification and rejection of bad workloads.

    use super::{PushScenario, PushWorkload, measure_push};

    #[test]
    fn only_the_overflow_scenarios_carry_a_policy() {
        assert_eq!(PushScenario::UncontendedHigh.policy(), None);
        assert_eq!(PushScenario::UncontendedLow.policy(), None);
        assert!(PushScenario::OverflowReturnError.policy().is_some());
        assert!(PushScenario::OverflowDrop.policy().is_some());
        assert!(PushScenario::OverflowWarnAndDrop.policy().is_some());
    }

    #[tokio::test]
    async fn uncontended_scenario_with_no_sends_is_rejected() {
        let error = measure_push(
            PushWorkload {
                scenario: PushScenario::UncontendedHigh,
                queue_capacity: 4,
                sends: 0,
            },
            1,
        )
        .await
        .expect_err("a workload with no sends must be rejected");

        assert!(
            error.contains("sends must be at least 1"),
            "unexpected: {error}"
        );
    }

    #[tokio::test]
    async fn overflow_drop_routes_every_rejected_frame_to_the_dead_letter_queue() {
        let measurement = measure_push(
            PushWorkload {
                scenario: PushScenario::OverflowDrop,
                queue_capacity: 4,
                sends: 8,
            },
            1,
        )
        .await
        .expect("a valid overflow workload must measure");

        // The queue holds 4 frames and is never drained, so all 8 sends meet a
        // full queue. `accepted` is not evidence that a frame was queued: a
        // drop policy returns `Ok` for a frame it routed elsewhere, so the DLQ
        // count is what proves every rejected frame was accounted for.
        assert_eq!(measurement.accepted, 8, "a drop policy reports success");
        assert_eq!(
            measurement.dead_lettered, 8,
            "every rejected frame must reach the dead-letter queue"
        );
        assert!(
            !measurement.reported_queue_full,
            "a drop policy must not surface a full queue as an error"
        );
        assert_eq!(
            measurement.drained, 0,
            "the queue is deliberately undrained"
        );
    }

    #[tokio::test]
    async fn overflow_return_error_rejects_without_dead_lettering() {
        let measurement = measure_push(
            PushWorkload {
                scenario: PushScenario::OverflowReturnError,
                queue_capacity: 4,
                sends: 8,
            },
            1,
        )
        .await
        .expect("a valid overflow workload must measure");

        assert_eq!(measurement.accepted, 0, "no frame is accepted");
        assert_eq!(
            measurement.dead_lettered, 0,
            "returning an error must not route to the dead-letter queue"
        );
        assert!(
            measurement.reported_queue_full,
            "the rejection must be reported"
        );
    }

    #[tokio::test]
    async fn uncontended_sends_are_accepted_and_drained() {
        let measurement = measure_push(
            PushWorkload {
                scenario: PushScenario::UncontendedHigh,
                queue_capacity: 4,
                sends: 32,
            },
            1,
        )
        .await
        .expect("a valid uncontended workload must measure");

        // An awaited send parks while the queue is full rather than failing, so
        // a draining consumer must observe exactly the frames that were sent.
        assert_eq!(measurement.accepted, 32);
        assert_eq!(measurement.drained, 32);
        assert_eq!(measurement.dead_lettered, 0);
        assert!(!measurement.reported_queue_full);
    }
}
