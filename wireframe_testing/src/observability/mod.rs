//! Test observability harness for capturing logs and metrics.
//!
//! This module provides [`ObservabilityHandle`], a unified guard that
//! combines log capture (via [`LoggerHandle`]) with metrics recording (via
//! [`DebuggingRecorder`]). Acquiring the handle locks the global logger
//! mutex and creates a fresh metrics recorder, so a single fixture is
//! enough to assert on both instrumentation axes.
//!
//! # Thread safety
//!
//! Logs are captured globally via a serializing mutex inherited from
//! [`LoggerHandle`]. Metrics are captured on the current thread via
//! [`metrics::with_local_recorder`]. Metric-emitting code must run on
//! the same thread as the handle for capture to work. For async tests,
//! use `#[tokio::test(flavor = "current_thread")]` or
//! `tokio::runtime::Runtime::new()?.block_on(...)`.
//!
//! # Snapshot semantics
//!
//! The underlying [`DebuggingRecorder`] uses atomic swap on snapshot,
//! meaning each call to [`ObservabilityHandle::snapshot`] drains the
//! counters. Call `snapshot()` once after recording metrics, then use
//! [`counter`](ObservabilityHandle::counter) and related methods to
//! query the captured values.
//!
//! # Examples
//!
//! ```no_run
//! use wireframe_testing::ObservabilityHandle;
//!
//! let mut obs = ObservabilityHandle::new();
//! obs.clear();
//!
//! metrics::with_local_recorder(obs.recorder(), || {
//!     wireframe::metrics::inc_codec_error("framing", "drop");
//! });
//!
//! obs.snapshot();
//! assert_eq!(
//!     obs.counter(
//!         wireframe::metrics::CODEC_ERRORS,
//!         &[("error_type", "framing"), ("recovery_policy", "drop")],
//!     ),
//!     1
//! );
//! ```

mod assertions;
mod labels;

use assertions::find_counter;
pub use labels::Labels;
use metrics::{SharedString, Unit};
use metrics_util::{
    CompositeKey,
    debugging::{DebugValue, DebuggingRecorder, Snapshotter},
};
use rstest::fixture;

use crate::logging::LoggerHandle;

/// A single entry from a metrics snapshot.
pub(crate) type SnapshotEntry = (CompositeKey, Option<Unit>, Option<SharedString>, DebugValue);

/// Combined log and metrics capture for test assertions.
///
/// `ObservabilityHandle` composes a [`LoggerHandle`] (for log capture)
/// with a [`DebuggingRecorder`] (for metrics capture). Acquiring the
/// handle locks the global logger mutex and creates a fresh thread-local
/// metrics recorder.
///
/// # Thread safety
///
/// Logs are captured globally via a serializing mutex. Metrics are
/// captured on the current thread via [`metrics::with_local_recorder`].
/// Metric-emitting code must run on the same thread as the handle for
/// capture to work.
///
/// # Examples
///
/// ```no_run
/// use wireframe_testing::ObservabilityHandle;
///
/// let mut obs = ObservabilityHandle::new();
/// obs.clear();
/// log::info!("captured");
/// ```
pub struct ObservabilityHandle {
    pub(crate) logger: LoggerHandle,
    recorder: DebuggingRecorder,
    snapshotter: Snapshotter,
    pub(crate) captured: Vec<SnapshotEntry>,
}

/// Acquire the global logger and create a fresh metrics recorder.
///
/// The returned handle holds the global logger mutex for its lifetime
/// and owns a [`DebuggingRecorder`] for metrics capture.
///
/// # Examples
///
/// ```no_run
/// use wireframe_testing::ObservabilityHandle;
///
/// let handle = ObservabilityHandle::new();
/// ```
impl ObservabilityHandle {
    /// Create a new observability handle.
    pub fn new() -> Self {
        let logger = LoggerHandle::new();
        let recorder = DebuggingRecorder::new();
        let snapshotter = recorder.snapshotter();
        Self {
            logger,
            recorder,
            snapshotter,
            captured: Vec::new(),
        }
    }

    /// Access the underlying log handle for direct log inspection.
    ///
    /// # Examples
    ///
    /// ```no_run
    /// use wireframe_testing::ObservabilityHandle;
    ///
    /// let mut obs = ObservabilityHandle::new();
    /// log::warn!("example");
    /// assert!(obs.logs().pop().is_some());
    /// ```
    pub fn logs(&mut self) -> &mut LoggerHandle { &mut self.logger }

    /// Access the metrics recorder for use with
    /// [`metrics::with_local_recorder`].
    ///
    /// # Examples
    ///
    /// ```no_run
    /// use wireframe_testing::ObservabilityHandle;
    ///
    /// let obs = ObservabilityHandle::new();
    /// metrics::with_local_recorder(obs.recorder(), || {
    ///     // Metrics emitted here are captured by the recorder.
    /// });
    /// ```
    pub fn recorder(&self) -> &DebuggingRecorder { &self.recorder }

    /// Take a snapshot of the current metrics state.
    ///
    /// This drains the underlying counters (atomic swap to zero) and
    /// stores the result for subsequent queries via
    /// [`counter`](Self::counter) and related methods.
    ///
    /// # Examples
    ///
    /// ```no_run
    /// use wireframe_testing::ObservabilityHandle;
    ///
    /// let mut obs = ObservabilityHandle::new();
    /// metrics::with_local_recorder(obs.recorder(), || {
    ///     wireframe::metrics::inc_connection_panics();
    /// });
    /// obs.snapshot();
    /// assert_eq!(
    ///     obs.counter_without_labels("wireframe_connection_panics_total"),
    ///     1
    /// );
    /// ```
    pub fn snapshot(&mut self) { self.captured = self.snapshotter.snapshot().into_vec(); }

    /// Clear all captured logs and drain the metrics snapshot.
    ///
    /// After calling `clear`, subsequent queries return zero counters and
    /// the log buffer is empty.
    ///
    /// # Examples
    ///
    /// ```no_run
    /// use wireframe_testing::ObservabilityHandle;
    ///
    /// let mut obs = ObservabilityHandle::new();
    /// obs.clear();
    /// ```
    pub fn clear(&mut self) {
        self.logger.clear();
        // Drain the snapshot so the next query starts fresh.
        let _ = self.snapshotter.snapshot();
        self.captured.clear();
    }

    /// Query a counter value by name and label pairs.
    ///
    /// Searches the most recent [`snapshot`](Self::snapshot) result.
    /// Returns 0 if the counter has not been recorded or no matching
    /// label combination is found.
    ///
    /// # Examples
    ///
    /// ```no_run
    /// use wireframe_testing::{ObservabilityHandle, observability::Labels};
    ///
    /// let mut obs = ObservabilityHandle::new();
    /// obs.snapshot();
    ///
    /// // Slice syntax (unchanged from previous API)
    /// let count = obs.counter(
    ///     "wireframe_frames_processed_total",
    ///     &[("direction", "inbound")],
    /// );
    ///
    /// // Builder syntax
    /// let count = obs.counter(
    ///     "wireframe_frames_processed_total",
    ///     Labels::new().with("direction", "inbound"),
    /// );
    /// ```
    #[must_use]
    pub fn counter(&self, name: &str, labels: impl Into<Labels>) -> u64 {
        let labels = labels.into();
        find_counter(&self.captured, name, &labels.as_str_pairs())
    }

    /// Query a counter value by name, aggregating across all label sets.
    ///
    /// Because no label filter is applied, the returned value is the sum
    /// of every series recorded under `name`, regardless of labels.
    ///
    /// # Examples
    ///
    /// ```no_run
    /// use wireframe_testing::ObservabilityHandle;
    ///
    /// let mut obs = ObservabilityHandle::new();
    /// obs.snapshot();
    /// let count = obs.counter_without_labels("wireframe_connection_panics_total");
    /// ```
    #[must_use]
    pub fn counter_without_labels(&self, name: &str) -> u64 { self.counter(name, Labels::new()) }

    /// Query the codec error counter by error type and recovery policy.
    ///
    /// This is a convenience wrapper around [`counter`](Self::counter)
    /// using the `wireframe_codec_errors_total` metric name.
    ///
    /// # Examples
    ///
    /// ```no_run
    /// use wireframe_testing::ObservabilityHandle;
    ///
    /// let mut obs = ObservabilityHandle::new();
    /// obs.snapshot();
    /// let count = obs.codec_error_counter("framing", "drop");
    /// ```
    #[must_use]
    pub fn codec_error_counter(&self, error_type: &str, recovery_policy: &str) -> u64 {
        self.counter(
            wireframe::metrics::CODEC_ERRORS,
            &[
                ("error_type", error_type),
                ("recovery_policy", recovery_policy),
            ],
        )
    }
}

/// rstest fixture returning an [`ObservabilityHandle`] for test
/// assertions.
///
/// # Examples
///
/// ```no_run
/// use wireframe_testing::observability::observability;
///
/// let mut obs = observability();
/// obs.clear();
/// ```
#[fixture]
pub fn observability() -> ObservabilityHandle {
    // Acquire the global logger and create a fresh metrics recorder
    ObservabilityHandle::new()
}
