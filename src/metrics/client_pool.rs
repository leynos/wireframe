//! Metrics for pooled client connection state and recovery.

// The import is used only by the real (non-loom) increment functions; a
// `loom` build compiles the tracing-only test matrix and would see it unused.
#[cfg(all(feature = "metrics", feature = "pool", not(loom)))]
use metrics::counter;

/// Name of the counter tracking pooled sockets discarded before safe reuse.
///
/// The bounded `reason` label separates sockets abandoned mid-operation from
/// sockets whose split-phase lease left a response unread.
///
/// ```plaintext
/// # HELP wireframe_pool_connection_discards_total Count of pooled sockets discarded by reason.
/// # TYPE wireframe_pool_connection_discards_total counter
/// wireframe_pool_connection_discards_total{reason="operation_incomplete"} 1
/// ```
pub const POOL_CONNECTION_DISCARDS: &str = "wireframe_pool_connection_discards_total";

/// Record a pooled socket discarded before its operation completed.
///
/// This function is a no-op when the `metrics` feature is disabled.
#[cfg(all(feature = "metrics", feature = "pool", not(loom)))]
pub(crate) fn inc_pool_incomplete_operation_discard() {
    counter!(POOL_CONNECTION_DISCARDS, "reason" => "operation_incomplete").increment(1);
}

/// Record a pooled socket discarded before its operation completed.
///
/// This function is a no-op when the `metrics` feature is disabled.
#[cfg(all(not(feature = "metrics"), feature = "pool", not(loom)))]
pub(crate) fn inc_pool_incomplete_operation_discard() {}

/// Record a pooled socket discarded after a split-phase lease left a response
/// unread.
///
/// This function is a no-op when the `metrics` feature is disabled.
#[cfg(all(feature = "metrics", feature = "pool", not(loom)))]
pub(crate) fn inc_pool_pending_response_discard() {
    counter!(POOL_CONNECTION_DISCARDS, "reason" => "response_pending").increment(1);
}

/// Record a pooled socket discarded after a split-phase lease left a response
/// unread.
///
/// This function is a no-op when the `metrics` feature is disabled.
#[cfg(all(not(feature = "metrics"), feature = "pool", not(loom)))]
pub(crate) fn inc_pool_pending_response_discard() {}

/// Name of the counter tracking client-pool acquisition attempts by mode and
/// outcome.
///
/// Both labels use fixed vocabularies, so pool activity cannot create new
/// series. The `mode` label is `"immediate"` for the synchronous fast path and
/// `"permit_race"` for the asynchronous race; the `outcome` label is one of
/// `"granted"`, `"exhausted"`, `"shutdown"`, or `"error"`.
///
/// ```plaintext
/// # HELP wireframe_pool_acquisitions_total Count of client-pool acquisition attempts by mode and outcome.
/// # TYPE wireframe_pool_acquisitions_total counter
/// wireframe_pool_acquisitions_total{mode="immediate",outcome="granted"} 3
/// wireframe_pool_acquisitions_total{mode="immediate",outcome="exhausted"} 1
/// wireframe_pool_acquisitions_total{mode="permit_race",outcome="error"} 1
/// ```
pub const POOL_ACQUISITIONS: &str = "wireframe_pool_acquisitions_total";

/// Bounded acquisition paths that decide how a pooled slot is selected.
///
/// Gated like [`inc_pool_acquisition`]: under `cfg(loom)` the client module
/// is compiled out, so leaving these labels visible would only be dead code.
#[cfg(all(feature = "pool", not(loom)))]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum PoolAcquisitionMode {
    /// The synchronous fast path tries each slot once without waiting.
    Immediate,
    /// The asynchronous path races one permit future per slot.
    PermitRace,
}

#[cfg(all(feature = "pool", not(loom)))]
impl PoolAcquisitionMode {
    /// Return the stable metric label for this acquisition path.
    pub(crate) const fn as_str(self) -> &'static str {
        match self {
            Self::Immediate => "immediate",
            Self::PermitRace => "permit_race",
        }
    }
}

/// Bounded outcomes of one client-pool acquisition attempt.
#[cfg(all(feature = "pool", not(loom)))]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum PoolAcquisitionOutcome {
    /// A slot granted capacity and the caller received a lease.
    Granted,
    /// Every slot was scanned without free capacity.
    Exhausted,
    /// The pool was shut down before or during the attempt, so no lease
    /// was granted.
    Shutdown,
    /// A slot wait or checkout failed with an error.
    Error,
}

#[cfg(all(feature = "pool", not(loom)))]
impl PoolAcquisitionOutcome {
    /// Return the stable metric label for this acquisition outcome.
    pub(crate) const fn as_str(self) -> &'static str {
        match self {
            Self::Granted => "granted",
            Self::Exhausted => "exhausted",
            Self::Shutdown => "shutdown",
            Self::Error => "error",
        }
    }
}

/// Record one acquisition attempt with fixed mode and outcome labels.
///
/// This function is a no-op when the `metrics` feature is disabled.
#[cfg(all(feature = "metrics", feature = "pool", not(loom)))]
pub(crate) fn inc_pool_acquisition(mode: PoolAcquisitionMode, outcome: PoolAcquisitionOutcome) {
    counter!(
        POOL_ACQUISITIONS,
        "mode" => mode.as_str(),
        "outcome" => outcome.as_str()
    )
    .increment(1);
}

/// Record one acquisition attempt with fixed mode and outcome labels.
///
/// This function is a no-op when the `metrics` feature is disabled.
#[cfg(all(not(feature = "metrics"), feature = "pool", not(loom)))]
pub(crate) fn inc_pool_acquisition(_mode: PoolAcquisitionMode, _outcome: PoolAcquisitionOutcome) {}

#[cfg(all(test, feature = "metrics", feature = "pool", not(loom)))]
mod tests {
    //! Tests for bounded client-pool discard and acquisition labels.

    use metrics_util::debugging::{DebugValue, DebuggingRecorder};
    use rstest::rstest;

    use super::{
        POOL_ACQUISITIONS,
        POOL_CONNECTION_DISCARDS,
        PoolAcquisitionMode,
        PoolAcquisitionOutcome,
        inc_pool_acquisition,
        inc_pool_incomplete_operation_discard,
        inc_pool_pending_response_discard,
    };

    #[rstest]
    #[case::incomplete_operation(
        "operation_incomplete",
        inc_pool_incomplete_operation_discard as fn(),
        "incomplete-operation discard metric was not recorded"
    )]
    #[case::pending_response(
        "response_pending",
        inc_pool_pending_response_discard as fn(),
        "pending-response discard metric was not recorded"
    )]
    fn discard_metric_uses_a_bounded_reason(
        #[case] reason: &str,
        #[case] increment: fn(),
        #[case] message: &str,
    ) {
        let recorder = DebuggingRecorder::new();
        let snapshotter = recorder.snapshotter();

        ::metrics::with_local_recorder(&recorder, increment);

        let found = snapshotter
            .snapshot()
            .into_vec()
            .iter()
            .any(|(key, _, _, value)| {
                key.key().name() == POOL_CONNECTION_DISCARDS
                    && key
                        .key()
                        .labels()
                        .any(|label| label.key() == "reason" && label.value() == reason)
                    && matches!(value, DebugValue::Counter(count) if *count == 1)
            });
        assert!(found, "{message}");
    }

    #[rstest]
    #[case::immediate_granted(PoolAcquisitionMode::Immediate, PoolAcquisitionOutcome::Granted)]
    #[case::immediate_shutdown(PoolAcquisitionMode::Immediate, PoolAcquisitionOutcome::Shutdown)]
    #[case::race_exhausted(PoolAcquisitionMode::PermitRace, PoolAcquisitionOutcome::Exhausted)]
    #[case::race_error(PoolAcquisitionMode::PermitRace, PoolAcquisitionOutcome::Error)]
    fn acquisition_metric_uses_bounded_mode_and_outcome(
        #[case] mode: PoolAcquisitionMode,
        #[case] outcome: PoolAcquisitionOutcome,
    ) {
        let recorder = DebuggingRecorder::new();
        let snapshotter = recorder.snapshotter();

        ::metrics::with_local_recorder(&recorder, || inc_pool_acquisition(mode, outcome));

        let found = snapshotter
            .snapshot()
            .into_vec()
            .iter()
            .any(|(key, _, _, value)| {
                key.key().name() == POOL_ACQUISITIONS
                    && key
                        .key()
                        .labels()
                        .any(|label| label.key() == "mode" && label.value() == mode.as_str())
                    && key
                        .key()
                        .labels()
                        .any(|label| label.key() == "outcome" && label.value() == outcome.as_str())
                    && matches!(value, DebugValue::Counter(count) if *count == 1)
            });
        assert!(
            found,
            "acquisition metric for {mode:?}/{outcome:?} was not recorded"
        );
    }
}
