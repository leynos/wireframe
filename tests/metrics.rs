//! Tests for `wireframe` metrics helpers.
//!
//! These tests verify that counters and gauges update as expected using
//! `metrics_util::debugging::DebuggingRecorder`.
use metrics_util::debugging::{DebugValue, DebuggingRecorder, Snapshotter};

fn snapshotter() -> (Snapshotter, DebuggingRecorder) {
    let recorder = DebuggingRecorder::new();
    let snapshotter = recorder.snapshotter();
    (snapshotter, recorder)
}

#[test]
fn outbound_frame_metric_increments() {
    let (snapshotter, recorder) = snapshotter();
    metrics::with_local_recorder(&recorder, || {
        wireframe::metrics::inc_frames(wireframe::metrics::Direction::Outbound);
    });

    let metrics = snapshotter.snapshot().into_vec();
    let found = metrics
        .iter()
        .any(|(k, ..)| k.key().name() == wireframe::metrics::FRAMES_PROCESSED);
    assert!(found, "frames_processed metric not recorded");
}

#[test]
fn inbound_frame_metric_increments() {
    let (snapshotter, recorder) = snapshotter();
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
    let (snapshotter, recorder) = snapshotter();
    metrics::with_local_recorder(&recorder, || {
        wireframe::metrics::inc_deser_errors();
    });

    let metrics = snapshotter.snapshot().into_vec();
    let found = metrics
        .iter()
        .any(|(k, ..)| k.key().name() == wireframe::metrics::ERRORS_TOTAL);
    assert!(found, "error metric not recorded");
}
