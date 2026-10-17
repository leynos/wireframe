//! Unit coverage for scenario classification and rejection of bad workloads.

use super::{PushScenario, PushWorkload, measure_push};

/// Every scenario the enum offers, so the classification tests cannot silently
/// skip one added later.
const ALL_SCENARIOS: [PushScenario; 8] = [
    PushScenario::UncontendedHigh,
    PushScenario::UncontendedLow,
    PushScenario::DrainedHigh,
    PushScenario::DrainedLow,
    PushScenario::BackpressureHigh,
    PushScenario::OverflowReturnError,
    PushScenario::OverflowDrop,
    PushScenario::OverflowWarnAndDrop,
];

#[test]
fn only_the_overflow_scenarios_carry_a_policy() {
    for scenario in ALL_SCENARIOS {
        let has_policy = scenario.policy().is_some();
        assert_eq!(
            has_policy,
            scenario.is_overflow(),
            "{} must carry a policy exactly when it is an overflow scenario",
            scenario.label()
        );
    }
}

#[test]
fn the_three_regimes_partition_the_scenarios() {
    // `run_once` dispatches on these predicates, so a scenario matching none of
    // them would be measured by whichever branch the `else` falls through to,
    // under a label whose assertions do not describe it.
    for scenario in ALL_SCENARIOS {
        let candidates = [
            scenario.is_overflow(),
            scenario.is_backpressure(),
            scenario.is_uncontended(),
            scenario.is_drained() && !scenario.is_uncontended(),
        ];
        assert_eq!(
            candidates.iter().filter(|flag| **flag).count(),
            1,
            "{} must belong to exactly one regime, got {candidates:?}",
            scenario.label()
        );
    }
}

#[tokio::test]
async fn an_uncontended_workload_larger_than_its_queue_is_rejected() {
    // The capacity relationship is what makes the no-wait claim provable, so a
    // workload violating it must be refused rather than measured and reported
    // as an uncontended finding it cannot support.
    let error = measure_push(
        PushWorkload {
            scenario: PushScenario::UncontendedHigh,
            queue_capacity: 4,
            sends: 8,
        },
        1,
    )
    .await
    .expect_err("a workload that can outrun its consumer must be rejected");

    assert!(
        error.contains("claims the uncontended regime"),
        "unexpected: {error}"
    );
}

#[tokio::test]
async fn a_backpressure_workload_its_queue_can_absorb_is_rejected() {
    // The reverse constraint: without more sends than slots nothing can park,
    // so the scenario would report zero waits under a back-pressure label.
    let error = measure_push(
        PushWorkload {
            scenario: PushScenario::BackpressureHigh,
            queue_capacity: 64,
            sends: 8,
        },
        1,
    )
    .await
    .expect_err("a workload that cannot park must be rejected");

    assert!(
        error.contains("no send can ever park"),
        "unexpected: {error}"
    );
}

#[tokio::test]
async fn a_drained_workload_smaller_than_its_queue_does_not_wait() {
    // The drained scenarios have no capacity constraint because whether the
    // producer outruns the consumer is the observation. This case pins the
    // other end of that: with room to spare, the observed wait count is zero
    // and the counter still shows the sends happened.
    let measurement = measure_push(
        PushWorkload {
            scenario: PushScenario::DrainedHigh,
            queue_capacity: 64,
            sends: 8,
        },
        1,
    )
    .await
    .expect("a drained workload must measure");

    assert_eq!(measurement.awaited_capacity_waits, 0);
    assert_eq!(measurement.capacity_probes, 8);
    assert_eq!(measurement.drained, 8);
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
async fn uncontended_sends_are_accepted_and_drained_without_waiting() {
    let measurement = measure_push(
        PushWorkload {
            scenario: PushScenario::UncontendedHigh,
            queue_capacity: 64,
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
    // The claim this scenario exists to support: no send waited. The probe
    // count is what stops that claim from being satisfied by measuring nothing.
    assert_eq!(
        measurement.awaited_capacity_waits, 0,
        "a draining consumer must keep the awaited path off the parked branch"
    );
    assert_eq!(
        measurement.capacity_probes, 32,
        "every send must have reached the channel"
    );
}

#[tokio::test]
async fn backpressure_scenario_forces_awaited_capacity_waits() {
    let measurement = measure_push(
        PushWorkload {
            scenario: PushScenario::BackpressureHigh,
            queue_capacity: 4,
            sends: 8,
        },
        1,
    )
    .await
    .expect("a valid back-pressure workload must measure");

    // The point of this scenario: the same awaited entry point as the
    // uncontended case, but the wait is now observable rather than absent.
    assert!(
        measurement.awaited_capacity_waits > 0,
        "the awaited path must park when nothing drains the queue"
    );
    assert_eq!(measurement.accepted, 8);
    assert_eq!(measurement.drained, 8);
    assert_eq!(measurement.dead_lettered, 0);
    // Awaited sends never report a full queue: they wait instead. This is the
    // gap the wait counters exist to fill, stated as an assertion.
    assert!(
        !measurement.reported_queue_full,
        "an awaited send reports a wait, never a full queue"
    );
}
