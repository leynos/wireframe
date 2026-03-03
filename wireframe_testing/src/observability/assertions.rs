//! Assertion methods for [`ObservabilityHandle`].
//!
//! These methods compare captured metrics and logs against expected
//! values, returning `Result<(), String>` so callers can use
//! `.expect(...)` for clear failure diagnostics.

use log::Level;
use metrics_util::debugging::DebugValue;

use super::{Labels, ObservabilityHandle, SnapshotEntry};

/// Assertion methods for metric counters and log records.
impl ObservabilityHandle {
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
    /// Drains the entire log buffer regardless of whether a match is
    /// found, preventing leftover records from leaking into subsequent
    /// assertions.  The `format_record` closure formats captured records
    /// for the error message on failure.
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
        let mut matched = false;
        let mut records = Vec::new();
        while let Some(record) = self.logger.pop() {
            if !matched && predicate(&record) {
                matched = true;
            } else {
                records.push(format_record(&record));
            }
        }
        if matched {
            Ok(())
        } else {
            Err(error_msg(&records))
        }
    }
}

/// Sum all counter series in `snapshot` whose name matches `name` and
/// whose labels are a superset of `labels`.
///
/// When `labels` is empty every series with the given name contributes,
/// making this an aggregate across all label combinations.  When
/// `labels` is non-empty only series containing every specified pair
/// are included, but the result is still the sum across all matching
/// series rather than just the first hit.
pub(super) fn find_counter(snapshot: &[SnapshotEntry], name: &str, labels: &[(&str, &str)]) -> u64 {
    snapshot
        .iter()
        .filter(|(key, ..)| key.key().name() == name)
        .filter(|(key, ..)| {
            labels
                .iter()
                .all(|(k, v)| key.key().labels().any(|l| l.key() == *k && l.value() == *v))
        })
        .filter_map(|(.., value)| match value {
            DebugValue::Counter(c) => Some(*c),
            _ => None,
        })
        .sum()
}
