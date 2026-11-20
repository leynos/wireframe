//! Behavioural coverage for the push-drop counter at its real loss site.
//!
//! The counter is the only aggregate evidence that a drop policy discarded a
//! frame, so each test drives a full lane through the public push API and
//! checks the sample the exporter would see.

use metrics::{SharedString, Unit};
use metrics_util::{
    CompositeKey,
    debugging::{DebugValue, DebuggingRecorder},
};
use tokio::sync::mpsc;

use super::super::{PushQueues, PushQueuesBuilder};
use crate::{
    metrics::PUSH_DROPS,
    push::{PushHandle, PushPolicy, PushPriority},
};

type MetricSample = (CompositeKey, Option<Unit>, Option<SharedString>, DebugValue);

/// Build a queue whose high-priority lane holds exactly one frame.
fn single_slot_queue(dlq: Option<mpsc::Sender<u8>>) -> (PushQueues<u8>, PushHandle<u8>) {
    PushQueuesBuilder::<u8>::default()
        .high_capacity(1)
        .low_capacity(1)
        .unlimited()
        .dlq(dlq)
        .build()
        .expect("queue builds")
}

/// Fill the high-priority lane so the next push meets a full queue.
fn fill_high_priority(handle: &PushHandle<u8>) {
    handle
        .try_push(1, PushPriority::High, PushPolicy::DropIfFull)
        .expect("the first push fits the lane");
}

/// Discard one frame on the full high-priority lane.
fn drop_on_a_full_lane(handle: &PushHandle<u8>, frame: u8) {
    handle
        .try_push(frame, PushPriority::High, PushPolicy::DropIfFull)
        .expect("a drop policy reports success even when it discards");
}

/// Whether `sample` counts one loss with the given outcome label.
fn sample_matches_drop(sample: &MetricSample, outcome: &str) -> bool {
    let (key, _, _, value) = sample;
    key.key().name() == PUSH_DROPS
        && key
            .key()
            .labels()
            .any(|label| label.key() == "policy" && label.value() == "drop_if_full")
        && key
            .key()
            .labels()
            .any(|label| label.key() == "outcome" && label.value() == outcome)
        && matches!(value, DebugValue::Counter(count) if *count == 1)
}

/// A full lane with nowhere to reroute the frame loses it, and that loss must
/// be visible to an operator even though the producer saw success.
#[test]
fn full_lane_without_a_dead_letter_queue_counts_the_loss() {
    let recorder = DebuggingRecorder::new();
    let snapshotter = recorder.snapshotter();

    ::metrics::with_local_recorder(&recorder, || {
        let (_queues, handle) = single_slot_queue(None);
        fill_high_priority(&handle);
        drop_on_a_full_lane(&handle, 2);
    });

    let metric_samples = snapshotter.snapshot().into_vec();
    assert_eq!(
        metric_samples.len(),
        1,
        "only the discarded frame is a loss"
    );
    assert!(
        metric_samples
            .iter()
            .any(|sample| sample_matches_drop(sample, "no_dlq")),
        "a full lane with no dead-letter queue must count the loss"
    );
}

/// The first overflow is rerouted, so it is not a loss; the second finds the
/// dead-letter queue full and is. Only the second may be counted.
#[test]
fn full_dead_letter_queue_counts_the_loss() {
    let recorder = DebuggingRecorder::new();
    let snapshotter = recorder.snapshotter();

    ::metrics::with_local_recorder(&recorder, || {
        let (dlq_tx, _dlq_rx) = mpsc::channel(1);
        let (_queues, handle) = single_slot_queue(Some(dlq_tx));
        fill_high_priority(&handle);
        drop_on_a_full_lane(&handle, 2);
        drop_on_a_full_lane(&handle, 3);
    });

    let metric_samples = snapshotter.snapshot().into_vec();
    assert_eq!(
        metric_samples.len(),
        1,
        "a frame the dead-letter queue accepted is not a loss"
    );
    assert!(
        metric_samples
            .iter()
            .any(|sample| sample_matches_drop(sample, "dlq_full")),
        "an overflow refused by a full dead-letter queue must count the loss"
    );
}

/// A dead-letter queue whose consumer has gone cannot take the frame, so the
/// loss is counted with the refusal's own outcome.
#[test]
fn closed_dead_letter_queue_counts_the_loss() {
    let recorder = DebuggingRecorder::new();
    let snapshotter = recorder.snapshotter();

    ::metrics::with_local_recorder(&recorder, || {
        let (dlq_tx, dlq_rx) = mpsc::channel(1);
        drop(dlq_rx);
        let (_queues, handle) = single_slot_queue(Some(dlq_tx));
        fill_high_priority(&handle);
        drop_on_a_full_lane(&handle, 2);
    });

    let metric_samples = snapshotter.snapshot().into_vec();
    assert_eq!(
        metric_samples.len(),
        1,
        "only the discarded frame is a loss"
    );
    assert!(
        metric_samples
            .iter()
            .any(|sample| sample_matches_drop(sample, "dlq_closed")),
        "an overflow refused by a closed dead-letter queue must count the loss"
    );
}
