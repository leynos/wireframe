//! Bounded cancellation reasons shared by server tracing and metrics.
//!
//! The server supervisor and accept loop format these labels through
//! `Display` even when the `metrics` feature is disabled, while the metric
//! counters use the feature-gated `as_str` helper.

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
    const fn label(self) -> &'static str {
        match self {
            Self::Graceful => "graceful",
            Self::Dropped => "dropped",
        }
    }
}

impl std::fmt::Display for ServerCancellationReason {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result { f.write_str(self.label()) }
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

    /// Tracing formats these labels through `Display` without the `metrics`
    /// feature, so the rendering must stay stable for that configuration.
    #[test]
    fn cancellation_reason_display_matches_bounded_labels() {
        assert_eq!(ServerCancellationReason::Graceful.to_string(), "graceful");
        assert_eq!(ServerCancellationReason::Dropped.to_string(), "dropped");
    }
}
