//! Unit coverage for scenario classification and rejection of bad workloads.

use super::{PushScenario, PushWorkload, measure_push};

#[test]
fn only_the_overflow_scenarios_carry_a_policy() {
    assert_eq!(PushScenario::UncontendedHigh.policy(), None);
    assert_eq!(PushScenario::UncontendedLow.policy(), None);
    assert!(PushScenario::OverflowReturnError.policy().is_some());
    assert!(PushScenario::OverflowDrop.policy().is_some());
    assert!(PushScenario::OverflowWarnAndDrop.policy().is_some());
}

#[tokio::test]
async fn uncontended_scenario_with_no_sends_is_rejected() {
    let error = measure_push(
        PushWorkload {
            scenario: PushScenario::UncontendedHigh,
            queue_capacity: 4,
            sends: 0,
        },
        1,
    )
    .await
    .expect_err("a workload with no sends must be rejected");

    assert!(
        error.contains("sends must be at least 1"),
        "unexpected: {error}"
    );
}

#[tokio::test]
async fn overflow_drop_routes_every_rejected_frame_to_the_dead_letter_queue() {
    let measurement = measure_push(
        PushWorkload {
            scenario: PushScenario::OverflowDrop,
            queue_capacity: 4,
            sends: 8,
        },
        1,
    )
    .await
    .expect("a valid overflow workload must measure");

    // The queue holds 4 frames and is never drained, so all 8 sends meet a
    // full queue. `accepted` is not evidence that a frame was queued: a
    // drop policy returns `Ok` for a frame it routed elsewhere, so the DLQ
    // count is what proves every rejected frame was accounted for.
    assert_eq!(measurement.accepted, 8, "a drop policy reports success");
    assert_eq!(
        measurement.dead_lettered, 8,
        "every rejected frame must reach the dead-letter queue"
    );
    assert!(
        !measurement.reported_queue_full,
        "a drop policy must not surface a full queue as an error"
    );
    assert_eq!(
        measurement.drained, 0,
        "the queue is deliberately undrained"
    );
}

#[tokio::test]
async fn overflow_return_error_rejects_without_dead_lettering() {
    let measurement = measure_push(
        PushWorkload {
            scenario: PushScenario::OverflowReturnError,
            queue_capacity: 4,
            sends: 8,
        },
        1,
    )
    .await
    .expect("a valid overflow workload must measure");

    assert_eq!(measurement.accepted, 0, "no frame is accepted");
    assert_eq!(
        measurement.dead_lettered, 0,
        "returning an error must not route to the dead-letter queue"
    );
    assert!(
        measurement.reported_queue_full,
        "the rejection must be reported"
    );
}

#[tokio::test]
async fn uncontended_sends_are_accepted_and_drained() {
    let measurement = measure_push(
        PushWorkload {
            scenario: PushScenario::UncontendedHigh,
            queue_capacity: 4,
            sends: 32,
        },
        1,
    )
    .await
    .expect("a valid uncontended workload must measure");

    // An awaited send parks while the queue is full rather than failing, so
    // a draining consumer must observe exactly the frames that were sent.
    assert_eq!(measurement.accepted, 32);
    assert_eq!(measurement.drained, 32);
    assert_eq!(measurement.dead_lettered, 0);
    assert!(!measurement.reported_queue_full);
}
