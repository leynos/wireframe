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
use rstest::{fixture, rstest};
use tokio::sync::mpsc;

use super::super::{PushConfigError, PushQueues, PushQueuesBuilder};
use crate::{
    metrics::PUSH_DROPS,
    push::{PushHandle, PushPolicy, PushPriority},
};

type MetricSample = (CompositeKey, Option<Unit>, Option<SharedString>, DebugValue);

/// A one-slot lane together with the dead-letter queue it overflows into.
struct DeadLetterQueue {
    /// Queue under test; kept alive for the scenario's duration.
    _queues: PushQueues<u8>,
    /// Handle that drives the queue.
    handle: PushHandle<u8>,
    /// Live dead-letter receiver, or `None` when the queue is closed.
    _receiver: Option<mpsc::Receiver<u8>>,
}

/// Build a one-slot high-priority lane with an optional dead-letter sender.
fn dead_letter_lane(
    dlq: Option<mpsc::Sender<u8>>,
) -> Result<(PushQueues<u8>, PushHandle<u8>), PushConfigError> {
    PushQueuesBuilder::<u8>::default()
        .high_capacity(1)
        .low_capacity(1)
        .unlimited()
        .dlq(dlq)
        .build()
}

/// Build a dead-letter queue, optionally with its consumer already gone.
fn dead_letter_queue(is_consumer_live: bool) -> Result<DeadLetterQueue, PushConfigError> {
    let (dlq_tx, dlq_rx) = mpsc::channel(1);
    let (queues, handle) = dead_letter_lane(Some(dlq_tx))?;
    Ok(DeadLetterQueue {
        _queues: queues,
        handle,
        _receiver: is_consumer_live.then_some(dlq_rx),
    })
}

/// Queue whose high-priority lane holds exactly one frame.
#[fixture]
fn single_slot_queue() -> Result<DeadLetterQueue, PushConfigError> {
    let (queues, handle) = dead_letter_lane(None)?;
    Ok(DeadLetterQueue {
        _queues: queues,
        handle,
        _receiver: None,
    })
}

/// One-slot lane whose dead-letter sender has a live receiver.
///
/// Note: `rustfmt` collapses this body onto one line, which trips
/// `unused_braces`, so keep `rustfmt::skip`.
#[rustfmt::skip]
#[fixture]
fn live_dead_letter_queue() -> Result<DeadLetterQueue, PushConfigError> {
    dead_letter_queue(true)
}

/// One-slot lane whose dead-letter receiver is already closed.
#[fixture]
fn closed_dead_letter_queue() -> Result<DeadLetterQueue, PushConfigError> {
    dead_letter_queue(false)
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

/// Run `scenario` against a fresh recorder and return the samples it emitted.
fn samples_from(scenario: impl FnOnce()) -> Vec<MetricSample> {
    let recorder = DebuggingRecorder::new();
    let snapshotter = recorder.snapshotter();
    ::metrics::with_local_recorder(&recorder, scenario);
    snapshotter.snapshot().into_vec()
}

/// Require exactly one loss sample, carrying `outcome`.
fn assert_single_loss(samples: &[MetricSample], outcome: &str) {
    assert_eq!(
        samples.len(),
        1,
        "exactly one frame may be lost, with outcome {outcome}: {samples:?}"
    );
    assert_eq!(
        samples
            .iter()
            .filter(|sample| sample_matches_drop(sample, outcome))
            .count(),
        1,
        "the lost frame must carry outcome {outcome}: {samples:?}"
    );
}

/// Fill the lane, overflow it `overflows` times, and require one loss sample.
fn assert_lost_frame(
    setup: Result<DeadLetterQueue, PushConfigError>,
    overflows: u8,
    outcome: &str,
) -> Result<(), PushConfigError> {
    let queue = setup?;
    let samples = samples_from(move || {
        fill_high_priority(&queue.handle);
        for frame in 2..2 + overflows {
            drop_on_a_full_lane(&queue.handle, frame);
        }
    });
    assert_single_loss(&samples, outcome);
    Ok(())
}

/// A full lane with nowhere to reroute the frame loses it, and that loss must
/// be visible to an operator even though the producer saw success.
#[rstest]
fn full_lane_without_a_dead_letter_queue_counts_the_loss(
    single_slot_queue: Result<DeadLetterQueue, PushConfigError>,
) -> Result<(), PushConfigError> {
    assert_lost_frame(single_slot_queue, 1, "no_dlq")
}

/// The first overflow is rerouted, so it is not a loss; the second finds the
/// dead-letter queue full and is. Only the second may be counted.
#[rstest]
fn full_dead_letter_queue_counts_the_loss(
    live_dead_letter_queue: Result<DeadLetterQueue, PushConfigError>,
) -> Result<(), PushConfigError> {
    assert_lost_frame(live_dead_letter_queue, 2, "dlq_full")
}

/// A dead-letter queue whose consumer has gone cannot take the frame, so the
/// loss is counted with the refusal's own outcome.
#[rstest]
fn closed_dead_letter_queue_counts_the_loss(
    closed_dead_letter_queue: Result<DeadLetterQueue, PushConfigError>,
) -> Result<(), PushConfigError> {
    assert_lost_frame(closed_dead_letter_queue, 1, "dlq_closed")
}
