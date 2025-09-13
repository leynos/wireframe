#![cfg(not(loom))]
//! Tests for `wireframe` metrics helpers.
//!
//! These tests verify that counters and gauges update as expected using
//! `metrics_util::debugging::DebuggingRecorder`.
use metrics_util::debugging::{DebugValue, DebuggingRecorder, Snapshotter};
use rstest::rstest;

/// Creates a debugging recorder and snapshotter for metrics testing.
fn debugging_recorder_setup() -> (Snapshotter, DebuggingRecorder) {
    let recorder = DebuggingRecorder::new();
    let snapshotter = recorder.snapshotter();
    (snapshotter, recorder)
}

#[test]
fn outbound_frame_metric_increments() {
    let (snapshotter, recorder) = debugging_recorder_setup();
    metrics::with_local_recorder(&recorder, || {
        wireframe::metrics::inc_frames(wireframe::metrics::Direction::Outbound);
    });

    let metrics = snapshotter.snapshot().into_vec();
    let found = metrics.iter().any(|(k, _, _, v)| {
        k.key().name() == wireframe::metrics::FRAMES_PROCESSED
            && k.key()
                .labels()
                .any(|l| l.key() == "direction" && l.value() == "outbound")
            && matches!(v, DebugValue::Counter(c) if *c > 0)
    });
    assert!(found, "outbound frames metric not recorded");
}

#[test]
fn inbound_frame_metric_increments() {
    let (snapshotter, recorder) = debugging_recorder_setup();
    metrics::with_local_recorder(&recorder, || {
        wireframe::metrics::inc_frames(wireframe::metrics::Direction::Inbound);
    });
    let metrics = snapshotter.snapshot().into_vec();
    let found = metrics.iter().any(|(k, _, _, v)| {
        k.key().name() == wireframe::metrics::FRAMES_PROCESSED
            && k.key()
                .labels()
                .any(|l| l.key() == "direction" && l.value() == "inbound")
            && matches!(v, DebugValue::Counter(c) if *c > 0)
    });

    assert!(found, "inbound frames metric not recorded");
}

#[test]
fn error_metric_increments() {
    let (snapshotter, recorder) = debugging_recorder_setup();
    metrics::with_local_recorder(&recorder, || {
        wireframe::metrics::inc_deser_errors();
    });

    let metrics = snapshotter.snapshot().into_vec();
    let found = metrics.iter().any(|(k, _, _, v)| {
        k.key().name() == wireframe::metrics::ERRORS_TOTAL
            && matches!(v, DebugValue::Counter(c) if *c > 0)
    });
    assert!(found, "error metric not recorded");
}

#[rstest]
#[case(1)]
#[case(2)]
fn inc_connection_panics_counts(#[case] expected: u64) {
    // Arrange
    let (snapshotter, recorder) = debugging_recorder_setup();

    // Act
    metrics::with_local_recorder(&recorder, || {
        (0..expected).for_each(|_| wireframe::metrics::inc_connection_panics());
    });

    // Assert
    assert_counter_eq(
        &snapshotter,
        wireframe::metrics::CONNECTION_PANICS,
        expected,
    );
}

fn assert_counter_eq(snapshotter: &Snapshotter, name: &str, expected: u64) {
    let metrics = snapshotter.snapshot().into_vec();
    assert!(
        metrics.iter().any(|(key, _, _, value)| {
            key.key().name() == name && matches!(value, DebugValue::Counter(c) if *c == expected)
        }),
        "expected {name} == {expected}, got {metrics:#?}"
    );
}
