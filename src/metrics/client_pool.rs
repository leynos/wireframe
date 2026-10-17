//! Metrics for pooled client connection state and recovery.

#[cfg(feature = "metrics")]
use metrics::counter;

/// Name of the counter tracking recovered client pool bookkeeping lock poison.
///
/// ```plaintext
/// # HELP wireframe_pool_bookkeeping_poison_recoveries_total Count of recovered client pool bookkeeping locks.
/// # TYPE wireframe_pool_bookkeeping_poison_recoveries_total counter
/// wireframe_pool_bookkeeping_poison_recoveries_total 1
/// ```
pub const POOL_BOOKKEEPING_POISON_RECOVERIES: &str =
    "wireframe_pool_bookkeeping_poison_recoveries_total";

/// Name of the counter tracking pooled sockets discarded after incomplete operations.
///
/// Bounded `reason` labels are `"operation_incomplete"` for cancellation or
/// unwinding during an operation, and `"response_pending"` when a split-phase
/// exchange leaves a response unread and its socket is scheduled for discard.
///
/// ```plaintext
/// # HELP wireframe_pool_connection_discards_total Count of pooled sockets discarded by reason.
/// # TYPE wireframe_pool_connection_discards_total counter
/// wireframe_pool_connection_discards_total{reason="operation_incomplete"} 1
/// ```
pub const POOL_CONNECTION_DISCARDS: &str = "wireframe_pool_connection_discards_total";

/// Record a recovered client pool bookkeeping lock poison event.
#[cfg(feature = "metrics")]
pub fn inc_pool_bookkeeping_poison_recoveries() {
    counter!(POOL_BOOKKEEPING_POISON_RECOVERIES).increment(1);
}

/// Record a recovered client pool bookkeeping lock poison event.
///
/// This function is a no-op when the `metrics` feature is disabled.
#[cfg(not(feature = "metrics"))]
pub fn inc_pool_bookkeeping_poison_recoveries() {}

/// Record a pooled socket discarded before its operation completed.
#[cfg(all(feature = "metrics", feature = "pool"))]
pub(crate) fn inc_pool_incomplete_operation_discard() {
    counter!(POOL_CONNECTION_DISCARDS, "reason" => "operation_incomplete").increment(1);
}

/// Record a pooled socket discarded before its operation completed.
///
/// This function is a no-op when the `metrics` feature is disabled.
#[cfg(all(not(feature = "metrics"), feature = "pool"))]
pub(crate) fn inc_pool_incomplete_operation_discard() {}

/// Record a pooled socket discarded after a split-phase lease left a response unread.
#[cfg(all(feature = "metrics", feature = "pool"))]
pub(crate) fn inc_pool_pending_response_discard() {
    counter!(POOL_CONNECTION_DISCARDS, "reason" => "response_pending").increment(1);
}

/// Record a pooled socket discarded after a split-phase lease left a response unread.
///
/// This function is a no-op when the `metrics` feature is disabled.
#[cfg(all(not(feature = "metrics"), feature = "pool"))]
pub(crate) fn inc_pool_pending_response_discard() {}

#[cfg(all(test, feature = "metrics", feature = "pool"))]
mod tests {
    //! Tests for bounded client-pool metric labels.

    use metrics_util::debugging::{DebugValue, DebuggingRecorder};
    use rstest::rstest;

    use super::{
        POOL_CONNECTION_DISCARDS,
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
}
