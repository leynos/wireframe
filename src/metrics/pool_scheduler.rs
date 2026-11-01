//! Metrics for the persistent client-pool scheduler task.
//!
//! Each pool owns exactly one scheduler task for its whole life, so these
//! series stay flat for a healthy pool and move only when that task stops
//! abnormally. Keeping them out of [`super::client_pool`] also keeps that
//! module inside the workspace's line limit.

#[cfg(all(feature = "metrics", feature = "pool", not(loom)))]
use metrics::counter;

/// Name of the counter tracking scheduler-task failures, labelled by pool.
///
/// A pool owns exactly one scheduler task, so this counter is flat for a
/// healthy pool and rises once if that task stops abnormally. The `pool` label
/// is the pool's process-local identity: it is constant for the pool's whole
/// lifetime, which keeps the series stable rather than growing per event.
///
/// ```plaintext
/// # HELP wireframe_pool_scheduler_failures_total Count of client-pool scheduler task failures by pool.
/// # TYPE wireframe_pool_scheduler_failures_total counter
/// wireframe_pool_scheduler_failures_total{pool="7"} 1
/// ```
pub const POOL_SCHEDULER_FAILURES: &str = "wireframe_pool_scheduler_failures_total";

/// Record one abnormally terminated scheduler task.
///
/// This function is a no-op when the `metrics` feature is disabled.
#[cfg(all(feature = "metrics", feature = "pool", not(loom)))]
pub(crate) fn inc_pool_scheduler_failure(pool_id: u64) {
    let pool = pool_id.to_string();
    counter!(POOL_SCHEDULER_FAILURES, "pool" => pool).increment(1);
}

/// Record one abnormally terminated scheduler task.
///
/// This function is a no-op when the `metrics` feature is disabled.
#[cfg(all(not(feature = "metrics"), feature = "pool", not(loom)))]
pub(crate) fn inc_pool_scheduler_failure(_pool_id: u64) {}

#[cfg(all(test, feature = "metrics", feature = "pool", not(loom)))]
mod tests {
    //! Tests for the pool-scheduler failure label.

    use metrics_util::debugging::{DebugValue, DebuggingRecorder};

    use super::{POOL_SCHEDULER_FAILURES, inc_pool_scheduler_failure};

    #[test]
    fn scheduler_failure_is_labelled_by_pool() {
        let recorder = DebuggingRecorder::new();
        let snapshotter = recorder.snapshotter();

        ::metrics::with_local_recorder(&recorder, || inc_pool_scheduler_failure(7));

        let found = snapshotter
            .snapshot()
            .into_vec()
            .iter()
            .any(|(key, _, _, value)| {
                key.key().name() == POOL_SCHEDULER_FAILURES
                    && key
                        .key()
                        .labels()
                        .any(|label| label.key() == "pool" && label.value() == "7")
                    && matches!(value, DebugValue::Counter(count) if *count == 1)
            });
        assert!(
            found,
            "the scheduler failure counter was not recorded for its pool"
        );
    }
}
