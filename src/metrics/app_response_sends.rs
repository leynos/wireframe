//! Metrics for outbound application-response sends.
//!
//! A `WireframeApp` handler response is serialized, wrapped by the
//! [`FrameCodec`], passed through the protocol `before_send` hook, and written
//! to the transport. The counter here separates bounded outcome classes so a
//! hook that pushes a frame past the codec limit is distinguishable from a
//! transport failure.
//!
//! [`FrameCodec`]: crate::codec::FrameCodec

#[cfg(feature = "metrics")]
use metrics::counter;

/// Name of the counter tracking outbound application-response send outcomes.
///
/// The `outcome` label uses a fixed vocabulary, so application data cannot
/// create new series.
///
/// ```plaintext
/// # HELP wireframe_app_response_sends_total Outbound application-response send outcomes.
/// # TYPE wireframe_app_response_sends_total counter
/// wireframe_app_response_sends_total{outcome="success"} 42
/// ```
pub const APP_RESPONSE_SENDS: &str = "wireframe_app_response_sends_total";

/// Bounded outcome of writing one application response frame.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum AppResponseSendOutcome {
    /// The frame reached the transport.
    Success,
    /// Serialization failed before the codec saw the frame.
    SerializationFailure,
    /// The codec rejected the frame, for example when a hook grew it past
    /// `max_frame_length`.
    CodecRejected,
    /// The transport write failed.
    TransportFailure,
}

impl AppResponseSendOutcome {
    /// Return the stable label for this outcome.
    ///
    /// The same fixed vocabulary serves the metric label and the `category`
    /// field of the send-failure warning, so operators can correlate the two
    /// without a second mapping. It is available whether or not the `metrics`
    /// feature is enabled.
    pub(crate) const fn as_str(self) -> &'static str {
        match self {
            Self::Success => "success",
            Self::SerializationFailure => "serialization_failure",
            Self::CodecRejected => "codec_rejected",
            Self::TransportFailure => "transport_failure",
        }
    }
}

/// Record one outbound application-response send outcome.
#[cfg(feature = "metrics")]
pub(crate) fn inc_app_response_send(outcome: AppResponseSendOutcome) {
    counter!(APP_RESPONSE_SENDS, "outcome" => outcome.as_str()).increment(1);
}

/// Record one outbound application-response send outcome.
///
/// This function is a no-op when the `metrics` feature is disabled.
#[cfg(not(feature = "metrics"))]
pub(crate) fn inc_app_response_send(_outcome: AppResponseSendOutcome) {}

#[cfg(all(test, feature = "metrics", not(loom)))]
mod tests {
    //! Tests for bounded application-response send metric labels.

    use metrics::{SharedString, Unit};
    use metrics_util::{
        CompositeKey,
        debugging::{DebugValue, DebuggingRecorder},
    };

    use super::{APP_RESPONSE_SENDS, AppResponseSendOutcome, inc_app_response_send};

    type MetricSample = (CompositeKey, Option<Unit>, Option<SharedString>, DebugValue);

    fn sample_matches_outcome(sample: &MetricSample, outcome: AppResponseSendOutcome) -> bool {
        let (key, _, _, value) = sample;
        if key.key().name() != APP_RESPONSE_SENDS || key.key().labels().count() != 1 {
            return false;
        }
        let has_outcome = key
            .key()
            .labels()
            .any(|label| label.key() == "outcome" && label.value() == outcome.as_str());
        has_outcome && matches!(value, DebugValue::Counter(count) if *count == 1)
    }

    #[test]
    fn app_response_sends_use_a_fixed_outcome_label() {
        let recorder = DebuggingRecorder::new();
        let snapshotter = recorder.snapshotter();
        let outcomes = [
            AppResponseSendOutcome::Success,
            AppResponseSendOutcome::SerializationFailure,
            AppResponseSendOutcome::CodecRejected,
            AppResponseSendOutcome::TransportFailure,
        ];

        ::metrics::with_local_recorder(&recorder, || {
            for outcome in outcomes {
                inc_app_response_send(outcome);
            }
        });

        let metric_samples = snapshotter.snapshot().into_vec();
        assert_eq!(metric_samples.len(), outcomes.len());
        for outcome in outcomes {
            assert!(
                metric_samples
                    .iter()
                    .any(|sample| sample_matches_outcome(sample, outcome)),
                "missing sample for outcome {outcome:?}"
            );
        }
    }
}
