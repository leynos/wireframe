//! Metrics for abandoned and refused push writes.
//!
//! A queued push can be abandoned before it reaches the transport: the
//! connection may stop while the write is pending, or the peer may stop being
//! observable and accept nothing inside the idle grace. Neither outcome is an
//! error the producer sees, so the counters here are the only aggregate signal
//! that queued output was dropped.
//!
//! A push refused by the rate limiter is likewise invisible to an operator when
//! the caller discards the error. It is counted separately, because a refusal
//! is back-pressure the application chose rather than a delivery failure.
//!
//! Both counters use a fixed vocabulary of labels, or none at all, so
//! application data cannot create new series. Neither is a per-connection
//! alert: a single stalled peer is an expected transient condition.

#[cfg(feature = "metrics")]
use metrics::counter;

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
        PUSH_RATE_LIMITED,
        PUSH_WRITES_ABANDONED,
        PushWriteAbandonReason,
        inc_push_rate_limited,
        inc_push_write_abandoned,
    };

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
}
