//! Metrics for preamble rejections raised after a successful decode.
//!
//! A preamble can decode cleanly and still be rejected by the success
//! callback, so these rejections are not decode failures and do not belong to
//! [`super::decode_rejections`]. Keeping them on their own counter lets
//! operators distinguish a malformed preamble from a peer that a
//! post-decode access-control callback declined.

#[cfg(feature = "metrics")]
use metrics::counter;

/// Name of the counter tracking post-decode preamble rejections by reason.
///
/// The `reason` label uses a fixed vocabulary, so peer input cannot create new
/// series.
///
/// ```plaintext
/// # HELP wireframe_preamble_rejections_total Post-decode preamble rejections by reason.
/// # TYPE wireframe_preamble_rejections_total counter
/// wireframe_preamble_rejections_total{reason="success_handler_error"} 1
/// ```
pub const PREAMBLE_REJECTIONS: &str = "wireframe_preamble_rejections_total";

/// Bounded reasons for rejecting a peer after its preamble decoded.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum PreambleRejectionReason {
    /// The success callback returned an error, so the connection failed closed
    /// before application dispatch.
    SuccessHandlerError,
}

impl PreambleRejectionReason {
    /// Return the stable metric label for this rejection reason.
    #[cfg(feature = "metrics")]
    pub(crate) const fn as_str(self) -> &'static str {
        match self {
            Self::SuccessHandlerError => "success_handler_error",
        }
    }
}

/// Record a post-decode preamble rejection using a fixed reason label.
#[cfg(feature = "metrics")]
pub(crate) fn inc_preamble_rejection(reason: PreambleRejectionReason) {
    counter!(PREAMBLE_REJECTIONS, "reason" => reason.as_str()).increment(1);
}

/// Record a post-decode preamble rejection using a fixed reason label.
///
/// This function is a no-op when the `metrics` feature is disabled.
#[cfg(not(feature = "metrics"))]
pub(crate) fn inc_preamble_rejection(_reason: PreambleRejectionReason) {}

#[cfg(all(test, feature = "metrics", not(loom)))]
mod tests {
    //! Tests for the post-decode preamble rejection metric.

    use metrics_util::{
        CompositeKey,
        debugging::{DebugValue, DebuggingRecorder},
    };

    use super::{PREAMBLE_REJECTIONS, PreambleRejectionReason, inc_preamble_rejection};

    fn has_label(key: &CompositeKey, expected_key: &str, expected_value: &str) -> bool {
        key.key()
            .labels()
            .any(|label| label.key() == expected_key && label.value() == expected_value)
    }

    #[test]
    fn preamble_rejections_use_fixed_reason_label() {
        let recorder = DebuggingRecorder::new();
        let snapshotter = recorder.snapshotter();

        ::metrics::with_local_recorder(&recorder, || {
            inc_preamble_rejection(PreambleRejectionReason::SuccessHandlerError);
        });

        let samples = snapshotter.snapshot().into_vec();
        assert_eq!(samples.len(), 1);
        assert!(
            samples.iter().any(|(key, _, _, value)| {
                key.key().name() == PREAMBLE_REJECTIONS
                    && key.key().labels().count() == 1
                    && has_label(key, "reason", "success_handler_error")
                    && matches!(value, DebugValue::Counter(count) if *count == 1)
            }),
            "expected one success-handler rejection sample, got {samples:?}",
        );
    }
}
