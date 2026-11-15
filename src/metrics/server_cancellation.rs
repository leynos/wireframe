//! Bounded cancellation reasons shared by server tracing and metrics.
//!
//! The server supervisor and accept loop record these labels through
//! [`ServerCancellationReason::label`] even when the `metrics` feature is
//! disabled, while the metric counters use the feature-gated `as_str`
//! helper.

/// Bounded reasons for server-supervisor cancellation metrics.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum ServerCancellationReason {
    /// Cancellation followed completion of the graceful-shutdown future.
    Graceful,
    /// Cancellation followed the server-supervisor future being dropped.
    Dropped,
}

impl ServerCancellationReason {
    /// Stable lowercase label shared by tracing and metrics.
    ///
    /// Tracing records this string whether or not the `metrics` feature is
    /// enabled, so subscribers can read the reason directly.
    pub(crate) const fn label(self) -> &'static str {
        match self {
            Self::Graceful => "graceful",
            Self::Dropped => "dropped",
        }
    }
}

#[cfg(feature = "metrics")]
impl ServerCancellationReason {
    /// Return the stable metric label value for this reason.
    pub(crate) const fn as_str(self) -> &'static str { self.label() }
}

#[cfg(test)]
mod tests {
    //! Tests for the metrics-independent tracing labels.

    use super::ServerCancellationReason;

    /// Tracing records these labels as strings without the `metrics` feature,
    /// so the rendering must stay stable for that configuration.
    #[test]
    fn cancellation_reason_labels_match_bounded_vocabulary() {
        assert_eq!(ServerCancellationReason::Graceful.label(), "graceful");
        assert_eq!(ServerCancellationReason::Dropped.label(), "dropped");
    }
}
