//! Metrics for dropped, abandoned, and refused pushes.
//!
//! A push can be lost before it ever reaches the transport. A non-blocking
//! queue policy discards a frame when its lane is full, and a queued frame can
//! be abandoned while a write is pending. Neither outcome is an error the
//! producer sees, so the counters here are the only aggregate signal that
//! output was dropped.
//!
//! A push refused by the rate limiter is likewise invisible to an operator when
//! the caller discards the error. It is counted separately, because a refusal
//! is back-pressure the application chose rather than a delivery failure.
//!
//! Every counter uses a fixed vocabulary of labels, or none at all, so
//! application data cannot create new series. None is a per-connection alert:
//! a stalled peer or a momentarily full queue is an expected transient
//! condition.

#[cfg(feature = "metrics")]
use metrics::counter;

use crate::push::PushPolicy;

/// Name of the counter tracking push frames abandoned before delivery.
///
/// The `reason` label is always either `"idle_grace_expired"`, when the peer
/// could not be observed and the write made no progress within the grace, or
/// `"shutdown"`, when the connection stopped while the write was pending.
///
/// ```plaintext
/// # HELP wireframe_push_writes_abandoned_total Push frames abandoned before delivery.
/// # TYPE wireframe_push_writes_abandoned_total counter
/// wireframe_push_writes_abandoned_total{reason="idle_grace_expired"} 2
/// wireframe_push_writes_abandoned_total{reason="shutdown"} 7
/// ```
pub const PUSH_WRITES_ABANDONED: &str = "wireframe_push_writes_abandoned_total";

/// Name of the counter tracking pushes refused by the connection's rate limiter.
///
/// A refusal is reported to the caller as
/// [`crate::push::PushError::RateLimited`]; this counter records it for an
/// operator who cannot see the caller's own errors. The counter has no labels,
/// so it never grows with connection identifiers or peer addresses.
///
/// ```plaintext
/// # HELP wireframe_push_rate_limited_total Pushes refused because the rate limiter had no permit.
/// # TYPE wireframe_push_rate_limited_total counter
/// wireframe_push_rate_limited_total 5
/// ```
pub const PUSH_RATE_LIMITED: &str = "wireframe_push_rate_limited_total";

/// Name of the counter tracking pushes discarded for good by their queue policy.
///
/// A non-blocking policy discards a frame when its lane is full. The frame is
/// lost only when there is no dead-letter queue to hand it to, or when the
/// dead-letter queue itself refuses it; a frame the dead-letter queue accepts
/// is not counted here. Both labels have a fixed vocabulary, so neither policy
/// names nor queue states can create unbounded series.
///
/// ```plaintext
/// # HELP wireframe_push_drops_total Pushes discarded without reaching any queue.
/// # TYPE wireframe_push_drops_total counter
/// wireframe_push_drops_total{policy="drop_if_full",outcome="no_dlq"} 4
/// wireframe_push_drops_total{policy="warn_and_drop_if_full",outcome="dlq_full"} 1
/// ```
pub const PUSH_DROPS: &str = "wireframe_push_drops_total";

/// Bounded reason a push policy discarded a frame without delivering it.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum PushDropOutcome {
    /// The queue has no dead-letter sink, so the frame had nowhere to go.
    NoDeadLetterQueue,
    /// The dead-letter queue exists but had no free capacity.
    DeadLetterFull,
    /// The dead-letter queue's receiver had been dropped.
    DeadLetterClosed,
}

#[cfg(feature = "metrics")]
impl PushDropOutcome {
    /// Return the stable label for this outcome.
    pub(crate) const fn as_str(self) -> &'static str {
        match self {
            Self::NoDeadLetterQueue => "no_dlq",
            Self::DeadLetterFull => "dlq_full",
            Self::DeadLetterClosed => "dlq_closed",
        }
    }
}

impl PushDropOutcome {
    /// Record one discarded push with this outcome and the caller's policy.
    pub(crate) fn record(self, policy: PushPolicy) { inc_push_drop(self, policy); }
}

/// Return the stable label for a queue policy.
///
/// `ReturnErrorIfFull` never discards a frame, so it never labels a sample;
/// it is mapped anyway so adding a policy cannot silently inherit a label.
#[cfg(feature = "metrics")]
const fn policy_label(policy: PushPolicy) -> &'static str {
    match policy {
        PushPolicy::ReturnErrorIfFull => "return_error_if_full",
        PushPolicy::DropIfFull => "drop_if_full",
        PushPolicy::WarnAndDropIfFull => "warn_and_drop_if_full",
    }
}

/// Record one push discarded for good, with its policy and the reason.
#[cfg(feature = "metrics")]
pub(crate) fn inc_push_drop(outcome: PushDropOutcome, policy: PushPolicy) {
    counter!(
        PUSH_DROPS,
        "policy" => policy_label(policy),
        "outcome" => outcome.as_str()
    )
    .increment(1);
}

/// Record one push discarded for good, with its policy and the reason.
///
/// This function is a no-op when the `metrics` feature is disabled.
#[cfg(not(feature = "metrics"))]
pub(crate) fn inc_push_drop(_outcome: PushDropOutcome, _policy: PushPolicy) {}

/// Bounded reason a push write stopped before reaching the transport.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum PushWriteAbandonReason {
    /// The peer could not be observed and accepted nothing within the grace.
    IdleGraceExpired,
    /// The connection stopped while the write was still pending.
    Shutdown,
}

impl PushWriteAbandonReason {
    /// Return the stable label for this reason.
    ///
    /// Unlike a purely metric-facing label this is also the value written into
    /// the structured abandonment warning, so it is always compiled.
    pub(crate) const fn as_str(self) -> &'static str {
        match self {
            Self::IdleGraceExpired => "idle_grace_expired",
            Self::Shutdown => "shutdown",
        }
    }

    /// Record one abandoned push write with this reason.
    pub(crate) fn record(self) { inc_push_write_abandoned(self); }
}

/// Record one abandoned push write.
#[cfg(feature = "metrics")]
pub(crate) fn inc_push_write_abandoned(reason: PushWriteAbandonReason) {
    counter!(PUSH_WRITES_ABANDONED, "reason" => reason.as_str()).increment(1);
}

/// Record one abandoned push write.
///
/// This function is a no-op when the `metrics` feature is disabled.
#[cfg(not(feature = "metrics"))]
pub(crate) fn inc_push_write_abandoned(_reason: PushWriteAbandonReason) {}

/// Record one push refused because the rate limiter had no permit.
#[cfg(feature = "metrics")]
pub(crate) fn inc_push_rate_limited() { counter!(PUSH_RATE_LIMITED).increment(1); }

/// Record one push refused because the rate limiter had no permit.
///
/// This function is a no-op when the `metrics` feature is disabled.
#[cfg(not(feature = "metrics"))]
pub(crate) fn inc_push_rate_limited() {}

#[cfg(all(test, feature = "metrics", not(loom)))]
mod tests {
    //! Tests for the bounded push-write metric labels.

    use metrics::{SharedString, Unit};
    use metrics_util::{
        CompositeKey,
        debugging::{DebugValue, DebuggingRecorder},
    };

    use super::{
        PUSH_DROPS,
        PUSH_RATE_LIMITED,
        PUSH_WRITES_ABANDONED,
        PushDropOutcome,
        PushWriteAbandonReason,
        inc_push_drop,
        inc_push_rate_limited,
        inc_push_write_abandoned,
    };
    use crate::push::PushPolicy;

    type MetricSample = (CompositeKey, Option<Unit>, Option<SharedString>, DebugValue);

    fn sample_matches_reason(sample: &MetricSample, reason: PushWriteAbandonReason) -> bool {
        let (key, _, _, value) = sample;
        if key.key().name() != PUSH_WRITES_ABANDONED || key.key().labels().count() != 1 {
            return false;
        }
        let has_reason = key
            .key()
            .labels()
            .any(|label| label.key() == "reason" && label.value() == reason.as_str());
        has_reason && matches!(value, DebugValue::Counter(count) if *count == 1)
    }

    fn sample_matches_rate_limited(sample: &MetricSample) -> bool {
        let (key, _, _, value) = sample;
        key.key().name() == PUSH_RATE_LIMITED
            && key.key().labels().count() == 0
            && matches!(value, DebugValue::Counter(count) if *count == 1)
    }

    fn sample_matches_drop(sample: &MetricSample, outcome: PushDropOutcome, policy: &str) -> bool {
        let (key, _, _, value) = sample;
        if key.key().name() != PUSH_DROPS || key.key().labels().count() != 2 {
            return false;
        }
        let has_policy = key
            .key()
            .labels()
            .any(|label| label.key() == "policy" && label.value() == policy);
        let has_outcome = key
            .key()
            .labels()
            .any(|label| label.key() == "outcome" && label.value() == outcome.as_str());
        has_policy && has_outcome && matches!(value, DebugValue::Counter(count) if *count == 1)
    }

    #[test]
    fn push_write_abandonment_uses_a_fixed_reason_label() {
        let recorder = DebuggingRecorder::new();
        let snapshotter = recorder.snapshotter();
        let reasons = [
            PushWriteAbandonReason::IdleGraceExpired,
            PushWriteAbandonReason::Shutdown,
        ];

        ::metrics::with_local_recorder(&recorder, || {
            for reason in reasons {
                inc_push_write_abandoned(reason);
            }
        });

        let metric_samples = snapshotter.snapshot().into_vec();
        assert_eq!(metric_samples.len(), reasons.len());
        for reason in reasons {
            assert!(
                metric_samples
                    .iter()
                    .any(|sample| sample_matches_reason(sample, reason)),
                "missing sample for reason {reason:?}"
            );
        }
    }

    #[test]
    fn rate_limited_refusals_use_an_unlabelled_counter() {
        let recorder = DebuggingRecorder::new();
        let snapshotter = recorder.snapshotter();

        ::metrics::with_local_recorder(&recorder, inc_push_rate_limited);

        let metric_samples = snapshotter.snapshot().into_vec();
        assert_eq!(metric_samples.len(), 1);
        assert!(
            metric_samples.iter().any(sample_matches_rate_limited),
            "missing the unlabelled rate-limited counter"
        );
    }

    /// The loss labels are bounded and distinct, so an operator can group by
    /// policy or outcome without the series growing with application data.
    #[test]
    fn push_drops_use_bounded_policy_and_outcome_labels() {
        let recorder = DebuggingRecorder::new();
        let snapshotter = recorder.snapshotter();
        // `ReturnErrorIfFull` is absent: that policy returns the refusal to
        // the caller instead of discarding the frame, so it loses nothing.
        let cases = [
            (
                PushDropOutcome::NoDeadLetterQueue,
                PushPolicy::DropIfFull,
                "drop_if_full",
            ),
            (
                PushDropOutcome::DeadLetterFull,
                PushPolicy::DropIfFull,
                "drop_if_full",
            ),
            (
                PushDropOutcome::DeadLetterClosed,
                PushPolicy::DropIfFull,
                "drop_if_full",
            ),
            (
                PushDropOutcome::DeadLetterFull,
                PushPolicy::WarnAndDropIfFull,
                "warn_and_drop_if_full",
            ),
        ];

        ::metrics::with_local_recorder(&recorder, || {
            for (outcome, policy, _) in cases {
                inc_push_drop(outcome, policy);
            }
        });

        let metric_samples = snapshotter.snapshot().into_vec();
        assert_eq!(metric_samples.len(), cases.len());
        for (outcome, _, policy) in cases {
            assert!(
                metric_samples
                    .iter()
                    .any(|sample| sample_matches_drop(sample, outcome, policy)),
                "missing sample for outcome {outcome:?} and policy {policy}"
            );
        }
    }
}
