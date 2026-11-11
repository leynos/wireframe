//! Metrics for prepared-connection response delivery.
//!
//! A streamed or multi-packet handler response is driven to completion by the
//! connection actor. It can end without delivering every frame when the
//! connection shuts down, or when the peer can no longer be observed and the
//! producer stops yielding. The counter here separates those bounded causes so
//! an operator can tell a stalled producer from an ordinary shutdown.

#[cfg(feature = "metrics")]
use metrics::counter;

/// Name of the counter tracking cancelled handler responses.
///
/// The `reason` label uses a fixed vocabulary, so application data cannot
/// create new series.
///
/// ```plaintext
/// # HELP wireframe_response_cancellations_total Cancelled handler responses.
/// # TYPE wireframe_response_cancellations_total counter
/// wireframe_response_cancellations_total{reason="idle_grace_expired"} 3
/// ```
pub const RESPONSE_CANCELLATIONS: &str = "wireframe_response_cancellations_total";

/// Bounded reason a handler response stopped before completing.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum ResponseCancellationReason {
    /// The peer could not be observed and the producer produced no frame
    /// within the idle grace.
    IdleGraceExpired,
    /// The connection shut down while the response was still active.
    Shutdown,
}

impl ResponseCancellationReason {
    /// Return the stable label for this reason.
    ///
    /// Only the metrics-enabled recorder reads the label, so a build without
    /// the feature would otherwise see this method as dead code.
    #[cfg(feature = "metrics")]
    pub(crate) const fn as_str(self) -> &'static str {
        match self {
            Self::IdleGraceExpired => "idle_grace_expired",
            Self::Shutdown => "shutdown",
        }
    }

    /// Record one cancellation with this reason.
    pub(crate) fn record(self) { inc_response_cancellation(self); }
}

/// Record one cancelled handler response.
#[cfg(feature = "metrics")]
pub(crate) fn inc_response_cancellation(reason: ResponseCancellationReason) {
    counter!(RESPONSE_CANCELLATIONS, "reason" => reason.as_str()).increment(1);
}

/// Record one cancelled handler response.
///
/// This function is a no-op when the `metrics` feature is disabled.
#[cfg(not(feature = "metrics"))]
pub(crate) fn inc_response_cancellation(_reason: ResponseCancellationReason) {}

#[cfg(all(test, feature = "metrics", not(loom)))]
mod tests {
    //! Tests for the bounded response-cancellation metric labels.

    use metrics::{SharedString, Unit};
    use metrics_util::{
        CompositeKey,
        debugging::{DebugValue, DebuggingRecorder},
    };

    use super::{RESPONSE_CANCELLATIONS, ResponseCancellationReason, inc_response_cancellation};

    type MetricSample = (CompositeKey, Option<Unit>, Option<SharedString>, DebugValue);

    fn sample_matches_reason(sample: &MetricSample, reason: ResponseCancellationReason) -> bool {
        let (key, _, _, value) = sample;
        if key.key().name() != RESPONSE_CANCELLATIONS || key.key().labels().count() != 1 {
            return false;
        }
        let has_reason = key
            .key()
            .labels()
            .any(|label| label.key() == "reason" && label.value() == reason.as_str());
        has_reason && matches!(value, DebugValue::Counter(count) if *count == 1)
    }

    #[test]
    fn response_cancellations_use_a_fixed_reason_label() {
        let recorder = DebuggingRecorder::new();
        let snapshotter = recorder.snapshotter();
        let reasons = [
            ResponseCancellationReason::IdleGraceExpired,
            ResponseCancellationReason::Shutdown,
        ];

        ::metrics::with_local_recorder(&recorder, || {
            for reason in reasons {
                inc_response_cancellation(reason);
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
}
