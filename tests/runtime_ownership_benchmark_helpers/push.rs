//! V-O4 — the push scenarios produce their documented outcomes.
//!
//! Each scenario names a different contract on a full queue: wait, return the
//! frame, or drop it. `reported_queue_full` is `false` for every awaited
//! scenario, so the wait counters carry the distinction the boolean cannot. The
//! oversized-queue control is what stops that distinction being vacuous: a
//! counter that was always non-zero would satisfy the back-pressure case alone.

use rstest::rstest;
use wireframe_testing::runtime_ownership_benchmarks::{PushScenario, PushWorkload, measure_push};

use super::harness::{ITERATIONS, block_on};

#[rstest]
#[case::high(PushScenario::UncontendedHigh)]
#[case::low(PushScenario::UncontendedLow)]
fn uncontended_sends_are_all_accepted_and_drained_without_waiting(#[case] scenario: PushScenario) {
    let sends = 32;
    let measurement = block_on(measure_push(
        PushWorkload {
            scenario,
            // Wider than the send count, which is what makes the no-wait claim a
            // property of the workload rather than of the scheduler's timing.
            queue_capacity: 64,
            sends,
        },
        ITERATIONS,
    ))
    .expect("uncontended push measurement failed");

    assert_eq!(
        measurement.accepted, sends,
        "every awaited send must be accepted"
    );
    assert_eq!(
        measurement.drained, sends,
        "a draining consumer must receive every frame, so none were lost"
    );
    assert_eq!(
        measurement.dead_lettered, 0,
        "the uncontended path must not route anything to the dead-letter queue"
    );
    assert!(
        !measurement.reported_queue_full,
        "an awaited send parks rather than reporting a full queue"
    );
    // The observable that makes "no back-pressure" a measurement rather than an
    // assumption: `reported_queue_full` is `false` for every awaited scenario,
    // so only the wait counters can tell the two regimes apart.
    assert_eq!(
        measurement.awaited_capacity_waits, 0,
        "a queue holding the whole run must not make any send park"
    );
    assert_eq!(
        measurement.capacity_probes, sends as u64,
        "every send must have reached the channel, or the zero above means nothing"
    );
}

/// The awaited path parked, which is what the back-pressure scenario exists for.
#[test]
fn backpressure_sends_park_for_capacity_and_are_eventually_drained() {
    let sends = 8;
    let measurement = block_on(measure_push(
        PushWorkload {
            scenario: PushScenario::BackpressureHigh,
            // Narrower than the send count, so the producer must park once the
            // queue fills. `validate` rejects the reverse.
            queue_capacity: 4,
            sends,
        },
        ITERATIONS,
    ))
    .expect("back-pressure push measurement failed");

    assert!(
        measurement.awaited_capacity_waits > 0,
        "nothing drains until a producer parks, so a wait must be observed"
    );
    assert_eq!(
        measurement.drained, sends,
        "the parked sends must complete once the drain starts"
    );
    assert!(
        !measurement.reported_queue_full,
        "the awaited path waits rather than reporting a full queue; that gap is why the wait \
         counters exist"
    );
}

/// The mirror case: a queue holding the whole run never pressures the producer.
///
/// Without this, the wait counter would never be shown to be discriminating —
/// a back-pressure test alone could be satisfied by a counter that is simply
/// always non-zero. The workload is a drained one at a capacity that fits it,
/// so the zero below is a property of the workload rather than of the
/// scheduler's timing.
#[test]
fn an_oversized_queue_leaves_the_awaited_path_unparked() {
    let measurement = block_on(measure_push(
        PushWorkload {
            scenario: PushScenario::DrainedHigh,
            queue_capacity: 64,
            sends: 8,
        },
        ITERATIONS,
    ))
    .expect("a workload the queue can absorb must still measure");

    assert_eq!(
        measurement.awaited_capacity_waits, 0,
        "a queue that holds every frame cannot make a producer wait"
    );
    assert_eq!(measurement.capacity_probes, 8);
}

#[rstest]
#[case::one(1)]
#[case::small(4)]
#[case::default(64)]
fn overflow_return_error_rejects_without_dead_lettering(#[case] queue_capacity: usize) {
    let measurement = block_on(measure_push(
        PushWorkload {
            scenario: PushScenario::OverflowReturnError,
            queue_capacity,
            sends: 8,
        },
        ITERATIONS,
    ))
    .expect("overflow return-error measurement failed");

    assert!(
        measurement.reported_queue_full,
        "`ReturnErrorIfFull` must surface `QueueFull` on a full queue"
    );
    assert_eq!(
        measurement.accepted, 0,
        "no send under `ReturnErrorIfFull` may report success against a full queue"
    );
    assert_eq!(
        measurement.dead_lettered, 0,
        "`ReturnErrorIfFull` returns the frame to the caller rather than the DLQ"
    );
}

#[rstest]
#[case::drop(PushScenario::OverflowDrop)]
#[case::warn_and_drop(PushScenario::OverflowWarnAndDrop)]
fn overflow_drop_routes_every_rejected_frame_to_the_dead_letter_queue(
    #[case] scenario: PushScenario,
) {
    let sends = 8;
    let measurement = block_on(measure_push(
        PushWorkload {
            scenario,
            queue_capacity: 4,
            sends,
        },
        ITERATIONS,
    ))
    .expect("overflow drop measurement failed");

    // A drop policy reports `Ok(())` for a frame it did *not* queue, so
    // `accepted` counts completed send paths rather than queued frames. The DLQ
    // count is what distinguishes the two, which is why both are asserted.
    assert_eq!(
        measurement.accepted, sends,
        "a drop policy reports that the send path completed"
    );
    assert_eq!(
        measurement.dead_lettered, sends,
        "every dropped frame must be observable on the dead-letter queue"
    );
    assert!(
        !measurement.reported_queue_full,
        "the drop policies swallow the full-queue condition rather than reporting it"
    );
    assert_eq!(
        measurement.drained, 0,
        "nothing was drained, so a non-zero count would mean a frame was lost"
    );
}
