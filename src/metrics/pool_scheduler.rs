//! Metrics for the persistent client-pool scheduler task.
//!
//! Each pool owns exactly one scheduler task for its whole life, so these
//! series stay flat for a healthy pool and move only when that task stops
//! abnormally. Keeping them out of [`super::client_pool`] also keeps that
//! module inside the workspace's line limit.

#[cfg(all(feature = "metrics", feature = "pool", not(loom)))]
use metrics::counter;

/// Name of the counter tracking scheduler-task failures.
///
/// A pool owns exactly one scheduler task, so this counter is flat for a
/// healthy pool and rises once if that task stops abnormally. It deliberately
/// carries no pool label: pool identities come from a monotonically increasing
/// allocator, so an application that replaces failed pools would create a new
/// time series per pool rather than incrementing one shared failure series.
/// The failing pool's identity stays in the accompanying tracing event, where
/// high-cardinality identifiers belong.
///
/// ```plaintext
/// # HELP wireframe_pool_scheduler_failures_total Count of client-pool scheduler task failures.
/// # TYPE wireframe_pool_scheduler_failures_total counter
/// wireframe_pool_scheduler_failures_total 1
/// ```
pub const POOL_SCHEDULER_FAILURES: &str = "wireframe_pool_scheduler_failures_total";

/// Record one abnormally terminated scheduler task.
///
/// This function is a no-op when the `metrics` feature is disabled.
#[cfg(all(feature = "metrics", feature = "pool", not(loom)))]
pub(crate) fn inc_pool_scheduler_failure() { counter!(POOL_SCHEDULER_FAILURES).increment(1); }

/// Record one abnormally terminated scheduler task.
///
/// This function is a no-op when the `metrics` feature is disabled.
#[cfg(all(not(feature = "metrics"), feature = "pool", not(loom)))]
pub(crate) fn inc_pool_scheduler_failure() {}

#[cfg(all(test, feature = "metrics", feature = "pool", not(loom)))]
mod tests {
    //! Tests for the pool-scheduler failure counter.

    use metrics_util::debugging::{DebugValue, DebuggingRecorder};

    use super::{POOL_SCHEDULER_FAILURES, inc_pool_scheduler_failure};

    #[test]
    fn scheduler_failure_is_recorded_without_a_pool_label() {
        let recorder = DebuggingRecorder::new();
        let snapshotter = recorder.snapshotter();

        ::metrics::with_local_recorder(&recorder, inc_pool_scheduler_failure);

        let entries = snapshotter.snapshot().into_vec();
        let failure = entries.iter().find_map(|(key, _, _, value)| {
            (key.key().name() == POOL_SCHEDULER_FAILURES).then_some((key, value))
        });
        let Some((key, value)) = failure else {
            panic!("the scheduler failure counter was not recorded");
        };
        assert!(
            key.key().labels().next().is_none(),
            "the failure counter must not carry an unbounded pool label"
        );
        assert!(
            matches!(value, DebugValue::Counter(count) if *count == 1),
            "the failure counter must increment once per failure"
        );
    }
}
