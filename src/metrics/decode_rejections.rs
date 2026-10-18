//! Metrics for bounded rejection of peer-controlled decoded data.

#[cfg(feature = "metrics")]
use metrics::counter;

/// Name of the counter tracking bounded decode rejections by path and reason.
///
/// Both labels use fixed vocabularies, so peer input cannot create new series.
///
/// ```plaintext
/// # HELP wireframe_decode_rejections_total Bounded decode rejections by path and reason.
/// # TYPE wireframe_decode_rejections_total counter
/// wireframe_decode_rejections_total{path="preamble",reason="limit_exceeded"} 1
/// ```
pub const DECODE_REJECTIONS: &str = "wireframe_decode_rejections_total";

/// Bounded processing paths that can reject peer-controlled decoded data.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum DecodeRejectionPath {
    /// Message compatibility decoding, including bincode and reassembly.
    Message,
    /// The optional Serde-backed bincode decoder.
    #[cfg(feature = "serializer-serde")]
    Serde,
    /// Connection preamble decoding.
    Preamble,
    /// The inbound envelope frame boundary.
    Envelope,
}

impl DecodeRejectionPath {
    /// Return the stable metric label for this decode path.
    pub(crate) const fn as_str(self) -> &'static str {
        match self {
            Self::Message => "message",
            #[cfg(feature = "serializer-serde")]
            Self::Serde => "serde",
            Self::Preamble => "preamble",
            Self::Envelope => "envelope",
        }
    }
}

/// Bounded reasons for rejecting decoded peer data.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum DecodeRejectionReason {
    /// Bincode's accounted-work limit was exceeded.
    LimitExceeded,
    /// A single-value envelope frame contained trailing bytes.
    TrailingBytes,
    /// A preamble failed for another decoding or read reason.
    DecodeError,
}

impl DecodeRejectionReason {
    /// Return the stable metric label for this rejection reason.
    pub(crate) const fn as_str(self) -> &'static str {
        match self {
            Self::LimitExceeded => "limit_exceeded",
            Self::TrailingBytes => "trailing_bytes",
            Self::DecodeError => "decode_error",
        }
    }
}

/// Record a bounded decode rejection using fixed path and reason labels.
#[cfg(feature = "metrics")]
pub(crate) fn inc_decode_rejection(path: DecodeRejectionPath, reason: DecodeRejectionReason) {
    counter!(
        DECODE_REJECTIONS,
        "path" => path.as_str(),
        "reason" => reason.as_str()
    )
    .increment(1);
}

/// Record a bounded decode rejection using fixed path and reason labels.
///
/// This function is a no-op when the `metrics` feature is disabled.
#[cfg(not(feature = "metrics"))]
pub(crate) fn inc_decode_rejection(_path: DecodeRejectionPath, _reason: DecodeRejectionReason) {}

#[cfg(all(test, feature = "metrics"))]
mod tests {
    //! Tests for bounded decode-rejection metric labels.

    use metrics_util::debugging::{DebugValue, DebuggingRecorder};

    use super::{
        DECODE_REJECTIONS,
        DecodeRejectionPath,
        DecodeRejectionReason,
        inc_decode_rejection,
    };

    #[test]
    fn decode_rejections_use_fixed_path_and_reason_labels() {
        let recorder = DebuggingRecorder::new();
        let snapshotter = recorder.snapshotter();
        let mut samples = vec![
            (
                DecodeRejectionPath::Message,
                DecodeRejectionReason::LimitExceeded,
            ),
            (
                DecodeRejectionPath::Preamble,
                DecodeRejectionReason::DecodeError,
            ),
            (
                DecodeRejectionPath::Envelope,
                DecodeRejectionReason::TrailingBytes,
            ),
        ];
        #[cfg(feature = "serializer-serde")]
        samples.push((
            DecodeRejectionPath::Serde,
            DecodeRejectionReason::LimitExceeded,
        ));

        ::metrics::with_local_recorder(&recorder, || {
            for &(path, reason) in &samples {
                inc_decode_rejection(path, reason);
            }
        });

        let metric_samples = snapshotter.snapshot().into_vec();
        assert_eq!(metric_samples.len(), samples.len());
        for &(path, reason) in &samples {
            assert!(metric_samples.iter().any(|(key, _, _, value)| {
                key.key().name() == DECODE_REJECTIONS
                    && key.key().labels().count() == 2
                    && key
                        .key()
                        .labels()
                        .any(|label| label.key() == "path" && label.value() == path.as_str())
                    && key
                        .key()
                        .labels()
                        .any(|label| label.key() == "reason" && label.value() == reason.as_str())
                    && matches!(value, DebugValue::Counter(count) if *count == 1)
            }));
        }
    }
}
