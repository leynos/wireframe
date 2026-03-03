//! Integration tests for the `wireframe_testing::observability` module.
//!
//! These tests verify that [`ObservabilityHandle`] captures logs and metrics
//! correctly and that the assertion helpers behave as documented.
//!
//! The global logger mutex means these tests must run serially. The test
//! binary is configured accordingly in `Cargo.toml`.
#![cfg(not(loom))]

use wireframe_testing::ObservabilityHandle;

#[test]
fn counter_returns_zero_for_unrecorded_metric() {
    let mut obs = ObservabilityHandle::new();
    obs.snapshot();
    assert_eq!(
        obs.counter("nonexistent_metric", []),
        0,
        "unrecorded counter should return 0"
    );
}

#[test]
fn counter_captures_incremented_metric() {
    let mut obs = ObservabilityHandle::new();
    metrics::with_local_recorder(obs.recorder(), || {
        wireframe::metrics::inc_connection_panics();
    });
    obs.snapshot();
    assert_eq!(
        obs.counter_without_labels(wireframe::metrics::CONNECTION_PANICS),
        1,
        "counter should be 1 after one increment"
    );
}

#[test]
fn counter_with_labels_filters_correctly() {
    let mut obs = ObservabilityHandle::new();
    metrics::with_local_recorder(obs.recorder(), || {
        wireframe::metrics::inc_codec_error("framing", "drop");
        wireframe::metrics::inc_codec_error("framing", "drop");
        wireframe::metrics::inc_codec_error("eof", "disconnect");
    });
    obs.snapshot();

    assert_eq!(
        obs.counter(
            wireframe::metrics::CODEC_ERRORS,
            [("error_type", "framing"), ("recovery_policy", "drop")],
        ),
        2,
        "framing/drop counter should be 2"
    );
    assert_eq!(
        obs.counter(
            wireframe::metrics::CODEC_ERRORS,
            [("error_type", "eof"), ("recovery_policy", "disconnect")],
        ),
        1,
        "eof/disconnect counter should be 1"
    );
}

#[test]
fn codec_error_counter_convenience_method() {
    let mut obs = ObservabilityHandle::new();
    metrics::with_local_recorder(obs.recorder(), || {
        wireframe::metrics::inc_codec_error("protocol", "drop");
    });
    obs.snapshot();
    assert_eq!(
        obs.codec_error_counter("protocol", "drop"),
        1,
        "codec error convenience should return 1"
    );
}

/// Creates an [`ObservabilityHandle`] with an optional metric increment,
/// then takes a snapshot so the handle is ready for assertions.
fn setup_obs_with_metric(record_metric: bool) -> ObservabilityHandle {
    let mut obs = ObservabilityHandle::new();
    if record_metric {
        metrics::with_local_recorder(obs.recorder(), || {
            wireframe::metrics::inc_connection_panics();
        });
    }
    obs.snapshot();
    obs
}

#[test]
fn assert_counter_passes_on_match() {
    let obs = setup_obs_with_metric(true);
    let result = obs.assert_counter(wireframe::metrics::CONNECTION_PANICS, [], 1);
    assert!(result.is_ok(), "assert_counter should pass on match");
}

#[test]
fn assert_counter_fails_on_mismatch() {
    let obs = setup_obs_with_metric(true);
    let result = obs.assert_counter(wireframe::metrics::CONNECTION_PANICS, [], 99);
    assert!(result.is_err(), "assert_counter should fail on mismatch");
}

#[test]
fn assert_no_metric_passes_when_absent() {
    let mut obs = ObservabilityHandle::new();
    obs.snapshot();
    let result = obs.assert_no_metric("nonexistent_metric");
    assert!(result.is_ok(), "assert_no_metric should pass when absent");
}

#[test]
fn assert_no_metric_fails_when_present() {
    let obs = setup_obs_with_metric(true);
    let result = obs.assert_no_metric(wireframe::metrics::CONNECTION_PANICS);
    assert!(
        result.is_err(),
        "assert_no_metric should fail when metric is present"
    );
}

#[test]
fn clear_resets_log_and_metric_state() {
    let mut obs = ObservabilityHandle::new();

    // Record a metric and a log entry
    metrics::with_local_recorder(obs.recorder(), || {
        wireframe::metrics::inc_connection_panics();
    });
    log::warn!("test log entry");

    // Verify the metric was captured
    obs.snapshot();
    assert!(
        obs.counter_without_labels(wireframe::metrics::CONNECTION_PANICS) > 0,
        "counter should be positive before clear"
    );

    obs.clear();
    obs.snapshot();

    // After clear, the counter snapshot is drained
    assert_eq!(
        obs.counter_without_labels(wireframe::metrics::CONNECTION_PANICS),
        0,
        "counter should be 0 after clear"
    );

    // Log buffer should also be empty
    assert!(
        obs.logs().pop().is_none(),
        "log buffer should be empty after clear"
    );
}

#[test]
fn log_assertion_finds_matching_substring() {
    let mut obs = ObservabilityHandle::new();
    obs.clear();
    log::warn!("codec frame dropped due to size");
    let result = obs.assert_log_contains("frame dropped");
    assert!(result.is_ok(), "should find matching log substring");
}

#[test]
fn log_assertion_fails_on_missing_substring() {
    let mut obs = ObservabilityHandle::new();
    obs.clear();
    let result = obs.assert_log_contains("this string is not present");
    assert!(
        result.is_err(),
        "should fail when substring is not in any log"
    );
}

#[test]
fn log_at_level_filters_correctly() {
    let mut obs = ObservabilityHandle::new();
    obs.clear();
    log::warn!("warning message for obs test");
    log::info!("info message for obs test");

    // Should find the warning
    let result = obs.assert_log_at_level(log::Level::Warn, "warning message");
    assert!(result.is_ok(), "should find warning log");

    // Re-emit for the negative test since assert_log_at_level drains
    log::info!("another info message for obs test");
    let result = obs.assert_log_at_level(log::Level::Warn, "another info");
    assert!(
        result.is_err(),
        "should not find info message at Warn level"
    );
}

#[test]
fn assert_codec_error_counter_convenience() {
    let mut obs = ObservabilityHandle::new();
    metrics::with_local_recorder(obs.recorder(), || {
        wireframe::metrics::inc_codec_error("framing", "drop");
        wireframe::metrics::inc_codec_error("framing", "drop");
        wireframe::metrics::inc_codec_error("framing", "drop");
    });
    obs.snapshot();

    let result = obs.assert_codec_error_counter("framing", "drop", 3);
    assert!(
        result.is_ok(),
        "codec error counter assertion should pass for 3"
    );
    let result = obs.assert_codec_error_counter("eof", "disconnect", 0);
    assert!(
        result.is_ok(),
        "unrecorded codec error counter should pass for 0"
    );
}
