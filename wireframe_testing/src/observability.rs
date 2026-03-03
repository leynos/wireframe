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

use log::Level;
use metrics::{SharedString, Unit};
use metrics_util::{
    CompositeKey,
    debugging::{DebugValue, DebuggingRecorder, Snapshotter},
};
use rstest::fixture;

use crate::logging::LoggerHandle;

/// A single entry from a metrics snapshot.
type SnapshotEntry = (CompositeKey, Option<Unit>, Option<SharedString>, DebugValue);

/// Owned metric label pairs for use with counter queries.
///
/// Provides a builder API and conversions from `&[(&str, &str)]` so
/// callers can use either literal slices or the builder pattern.
///
/// # Examples
///
/// ```no_run
/// use wireframe_testing::observability::Labels;
///
/// // Builder pattern
/// let labels = Labels::new()
///     .with("error_type", "framing")
///     .with("recovery_policy", "drop");
///
/// // From a literal array
/// let labels: Labels = [("error_type", "framing"), ("recovery_policy", "drop")].into();
/// ```
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Labels(Vec<(String, String)>);

impl Labels {
    /// Create an empty label set.
    #[must_use]
    pub fn new() -> Self { Self::default() }

    /// Append a key-value pair, returning `self` for chaining.
    #[must_use]
    pub fn with(mut self, key: impl Into<String>, value: impl Into<String>) -> Self {
        self.0.push((key.into(), value.into()));
        self
    }

    /// Convert to temporary borrowed pairs for internal queries.
    pub(crate) fn as_str_pairs(&self) -> Vec<(&str, &str)> {
        self.0
            .iter()
            .map(|(k, v)| (k.as_str(), v.as_str()))
            .collect()
    }
}

impl From<&[(&str, &str)]> for Labels {
    fn from(pairs: &[(&str, &str)]) -> Self {
        Self(
            pairs
                .iter()
                .map(|(k, v)| ((*k).to_owned(), (*v).to_owned()))
                .collect(),
        )
    }
}

impl<const N: usize> From<[(&str, &str); N]> for Labels {
    fn from(pairs: [(&str, &str); N]) -> Self {
        Self(
            pairs
                .into_iter()
                .map(|(k, v)| (k.to_owned(), v.to_owned()))
                .collect(),
        )
    }
}

impl<const N: usize> From<&[(&str, &str); N]> for Labels {
    fn from(pairs: &[(&str, &str); N]) -> Self {
        Self(
            pairs
                .iter()
                .map(|(k, v)| ((*k).to_owned(), (*v).to_owned()))
                .collect(),
        )
    }
}

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
    logger: LoggerHandle,
    recorder: DebuggingRecorder,
    snapshotter: Snapshotter,
    captured: Vec<SnapshotEntry>,
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

    /// Query a counter value by name without label filtering.
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

    /// Assert a counter matches the expected value.
    ///
    /// Returns `Err` with a descriptive message if the counter does not
    /// match.
    ///
    /// # Errors
    ///
    /// Returns `Err` when the observed counter value differs from
    /// `expected`.
    ///
    /// # Examples
    ///
    /// ```no_run
    /// use wireframe_testing::{ObservabilityHandle, observability::Labels};
    ///
    /// let mut obs = ObservabilityHandle::new();
    /// obs.snapshot();
    ///
    /// // Slice syntax
    /// obs.assert_counter("wireframe_connection_panics_total", &[], 0)
    ///     .expect("counter should be zero");
    ///
    /// // Builder syntax
    /// obs.assert_counter(
    ///     "wireframe_codec_errors_total",
    ///     Labels::new().with("error_type", "framing"),
    ///     0,
    /// )
    /// .expect("counter should be zero");
    /// ```
    pub fn assert_counter(
        &self,
        name: &str,
        labels: impl Into<Labels>,
        expected: u64,
    ) -> Result<(), String> {
        let labels = labels.into();
        let actual = find_counter(&self.captured, name, &labels.as_str_pairs());
        if actual == expected {
            Ok(())
        } else {
            Err(format!(
                "counter {name} with labels {labels:?}: expected {expected}, got {actual}"
            ))
        }
    }

    /// Assert no metric with the given name exists in the snapshot.
    ///
    /// # Errors
    ///
    /// Returns `Err` when a metric with the given name is found.
    ///
    /// # Examples
    ///
    /// ```no_run
    /// use wireframe_testing::ObservabilityHandle;
    ///
    /// let mut obs = ObservabilityHandle::new();
    /// obs.snapshot();
    /// obs.assert_no_metric("nonexistent_metric")
    ///     .expect("metric should not exist");
    /// ```
    pub fn assert_no_metric(&self, name: &str) -> Result<(), String> {
        let found = self
            .captured
            .iter()
            .any(|(key, ..)| key.key().name() == name);
        if found {
            Err(format!("expected no metric named {name}, but found one"))
        } else {
            Ok(())
        }
    }

    /// Assert the codec error counter matches the expected value.
    ///
    /// Convenience wrapper combining
    /// [`codec_error_counter`](Self::codec_error_counter) with an
    /// equality check.
    ///
    /// # Errors
    ///
    /// Returns `Err` when the observed counter value differs from
    /// `expected`.
    ///
    /// # Examples
    ///
    /// ```no_run
    /// use wireframe_testing::ObservabilityHandle;
    ///
    /// let mut obs = ObservabilityHandle::new();
    /// obs.snapshot();
    /// obs.assert_codec_error_counter("framing", "drop", 0)
    ///     .expect("no framing errors expected");
    /// ```
    pub fn assert_codec_error_counter(
        &self,
        error_type: &str,
        recovery_policy: &str,
        expected: u64,
    ) -> Result<(), String> {
        let actual = self.codec_error_counter(error_type, recovery_policy);
        if actual == expected {
            Ok(())
        } else {
            Err(format!(
                "codec error counter (error_type={error_type}, \
                 recovery_policy={recovery_policy}): expected {expected}, got {actual}"
            ))
        }
    }

    /// Assert any captured log contains the given substring.
    ///
    /// Drains the log buffer and checks each record. The buffer is
    /// consumed by this operation.
    ///
    /// # Errors
    ///
    /// Returns `Err` when no captured log record contains `substring`.
    ///
    /// # Examples
    ///
    /// ```no_run
    /// use wireframe_testing::ObservabilityHandle;
    ///
    /// let mut obs = ObservabilityHandle::new();
    /// log::warn!("codec frame dropped");
    /// obs.assert_log_contains("frame dropped")
    ///     .expect("should find log message");
    /// ```
    pub fn assert_log_contains(&mut self, substring: &str) -> Result<(), String> {
        self.assert_log_matches(
            |record| record.args().to_string().contains(substring),
            |record| record.args().to_string(),
            |records| format!("no log record containing {substring:?}; captured: {records:?}"),
        )
    }

    /// Assert a log at the given level contains the substring.
    ///
    /// Drains the log buffer and checks records matching both `level` and
    /// `substring`. The buffer is consumed by this operation.
    ///
    /// # Errors
    ///
    /// Returns `Err` when no captured log at `level` contains `substring`.
    ///
    /// # Examples
    ///
    /// ```no_run
    /// use wireframe_testing::ObservabilityHandle;
    ///
    /// let mut obs = ObservabilityHandle::new();
    /// log::warn!("codec frame dropped");
    /// obs.assert_log_at_level(log::Level::Warn, "frame dropped")
    ///     .expect("should find warning");
    /// ```
    pub fn assert_log_at_level(&mut self, level: Level, substring: &str) -> Result<(), String> {
        self.assert_log_matches(
            |record| record.level() == level && record.args().to_string().contains(substring),
            |record| format!("[{}] {}", record.level(), record.args()),
            |records| format!("no {level} log containing {substring:?}; captured: {records:?}"),
        )
    }

    /// Helper to search logs matching a predicate.
    ///
    /// Drains the log buffer and returns Ok if any record satisfies the
    /// predicate. The `format_record` closure formats captured records
    /// for the error message.
    fn assert_log_matches<F, G>(
        &mut self,
        predicate: F,
        format_record: G,
        error_msg: impl FnOnce(&[String]) -> String,
    ) -> Result<(), String>
    where
        F: Fn(&logtest::Record) -> bool,
        G: Fn(&logtest::Record) -> String,
    {
        let mut records = Vec::new();
        while let Some(record) = self.logger.pop() {
            if predicate(&record) {
                return Ok(());
            }
            records.push(format_record(&record));
        }
        Err(error_msg(&records))
    }
}

/// Search a snapshot vector for a counter matching `name` and all labels.
fn find_counter(snapshot: &[SnapshotEntry], name: &str, labels: &[(&str, &str)]) -> u64 {
    for (key, _unit, _desc, value) in snapshot {
        if key.key().name() != name {
            continue;
        }
        let all_match = labels
            .iter()
            .all(|(k, v)| key.key().labels().any(|l| l.key() == *k && l.value() == *v));
        if all_match {
            if let DebugValue::Counter(c) = value {
                return *c;
            }
        }
    }
    0
}

/// rstest fixture returning an [`ObservabilityHandle`] for test assertions.
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
