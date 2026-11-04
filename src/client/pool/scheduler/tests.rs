//! Coverage for the scheduler's owned waiter queues and bounded admission.

use std::collections::VecDeque;

use proptest::{prelude::*, test_runner::Config as ProptestConfig};
use rstest::rstest;
use tokio::sync::oneshot;

use super::{
    state::{SchedulerState, WaiterAdmission, WaiterEnqueueResult},
    *,
};
use crate::{client::ClientError, serializer::BincodeSerializer};

type TestState = SchedulerState<BincodeSerializer, (), ()>;
type TestLease = super::super::PooledClientLease<BincodeSerializer, (), ()>;
type TestReceiver = oneshot::Receiver<Result<TestLease, ClientError>>;

fn admission(fairness_policy: PoolFairnessPolicy, max_waiters: usize) -> WaiterAdmission {
    WaiterAdmission {
        fairness_policy,
        max_waiters,
    }
}

/// Build a state with one queued waiter and return its receiver.
fn enqueue_one(
    state: &mut TestState,
    handle_id: u64,
    policy: PoolFairnessPolicy,
    max_waiters: usize,
) -> TestReceiver {
    let (sender, receiver) = oneshot::channel();
    let result = state.enqueue_waiter(handle_id, sender, admission(policy, max_waiters));
    assert!(
        matches!(result, WaiterEnqueueResult::Enqueued),
        "the first waiter must be admitted"
    );
    receiver
}

#[rstest]
#[case(PoolFairnessPolicy::Fifo)]
#[case(PoolFairnessPolicy::RoundRobin)]
fn enqueue_registers_a_handle_the_first_time_it_appears(#[case] policy: PoolFairnessPolicy) {
    let mut state = TestState::new();
    let _receiver = enqueue_one(&mut state, 42, policy, 8);

    let waiter = state.take_next_waiter(policy);
    assert!(
        waiter.is_some(),
        "lazy registration must make the request selectable"
    );
    assert!(
        !state.has_queued_waiter(42),
        "assertion failed: !state.has_queued_waiter(42)"
    );
}

#[rstest]
#[case(PoolFairnessPolicy::Fifo)]
#[case(PoolFairnessPolicy::RoundRobin)]
fn deregistering_a_handle_twice_is_harmless(#[case] policy: PoolFairnessPolicy) {
    let mut state = TestState::new();
    let _receiver = enqueue_one(&mut state, 7, policy, 8);

    state.deregister_handle(7);
    state.deregister_handle(7);

    assert_eq!(
        state.waiter_count(),
        0,
        "state.waiter_count() should equal 0"
    );
    assert!(
        state.take_next_waiter(policy).is_none(),
        "assertion failed: state.take_next_waiter(policy).is_none()"
    );
}

#[rstest]
#[case(PoolFairnessPolicy::Fifo)]
#[case(PoolFairnessPolicy::RoundRobin)]
fn enqueue_rejects_waiters_above_the_configured_limit(#[case] policy: PoolFairnessPolicy) {
    let mut state = TestState::new();
    let _first = enqueue_one(&mut state, 1, policy, 2);
    let _second = enqueue_one(&mut state, 2, policy, 2);

    let (sender, receiver) = oneshot::channel();
    assert!(
        matches!(
            state.enqueue_waiter(1, sender, admission(policy, 2)),
            WaiterEnqueueResult::Saturated { .. }
        ),
        "a waiter beyond the bound must be handed back, not queued"
    );
    drop(receiver);
    assert_eq!(
        state.waiter_count(),
        2,
        "state.waiter_count() should equal 2"
    );
}

/// Frees admission capacity when a caller cancels before its waiter is served.
#[rstest]
#[case(PoolFairnessPolicy::Fifo)]
#[case(PoolFairnessPolicy::RoundRobin)]
fn enqueue_prunes_cancelled_waiters_before_counting(#[case] policy: PoolFairnessPolicy) {
    let mut state = TestState::new();
    let cancelled = enqueue_one(&mut state, 1, policy, 2);
    let _live = enqueue_one(&mut state, 2, policy, 2);
    drop(cancelled);

    let _replacement = enqueue_one(&mut state, 1, policy, 2);
    assert_eq!(
        state.waiter_count(),
        2,
        "state.waiter_count() should equal 2"
    );
}

#[rstest]
#[case(PoolFairnessPolicy::Fifo)]
#[case(PoolFairnessPolicy::RoundRobin)]
fn prune_removes_every_abandoned_waiter(#[case] policy: PoolFairnessPolicy) {
    let mut state = TestState::new();
    let first = enqueue_one(&mut state, 1, policy, 8);
    let second = enqueue_one(&mut state, 2, policy, 8);
    drop((first, second));

    state.prune_cancelled_waiters();

    assert_eq!(
        state.waiter_count(),
        0,
        "state.waiter_count() should equal 0"
    );
    assert!(
        !state.has_live_waiters(),
        "assertion failed: !state.has_live_waiters()"
    );
}

#[rstest]
#[case(PoolFairnessPolicy::Fifo)]
#[case(PoolFairnessPolicy::RoundRobin)]
fn deregister_handle_purges_every_entry_for_that_handle(#[case] policy: PoolFairnessPolicy) {
    let mut state = TestState::new();
    let _removed = enqueue_one(&mut state, 1, policy, 8);
    let _kept = enqueue_one(&mut state, 2, policy, 8);

    state.deregister_handle(1);

    assert!(
        !state.has_queued_waiter(1),
        "assertion failed: !state.has_queued_waiter(1)"
    );
    assert!(
        state.take_next_waiter(policy).is_some(),
        "assertion failed: state.take_next_waiter(policy).is_some()"
    );
    assert!(
        state.take_next_waiter(policy).is_none(),
        "deregistered handle entries must be removed eagerly"
    );
}

#[rstest]
#[case(PoolFairnessPolicy::Fifo)]
#[case(PoolFairnessPolicy::RoundRobin)]
fn take_next_waiter_skips_a_caller_that_went_away(#[case] policy: PoolFairnessPolicy) {
    let mut state = TestState::new();
    let abandoned = enqueue_one(&mut state, 1, policy, 8);
    let _live = enqueue_one(&mut state, 1, policy, 8);
    drop(abandoned);

    let selected = state
        .take_next_waiter(policy)
        .expect("the live waiter must still be selected");
    assert!(
        !selected.is_closed(),
        "a lease must never be handed to a caller that stopped listening"
    );
    assert!(
        state.take_next_waiter(policy).is_none(),
        "assertion failed: state.take_next_waiter(policy).is_none()"
    );
}

#[test]
fn round_robin_gives_each_handle_a_turn_before_repeating() {
    let mut state = TestState::new();
    let _first = enqueue_one(&mut state, 1, PoolFairnessPolicy::RoundRobin, 8);
    let _second = enqueue_one(&mut state, 2, PoolFairnessPolicy::RoundRobin, 8);

    let first_turn = state.take_next_waiter(PoolFairnessPolicy::RoundRobin);
    let second_turn = state.take_next_waiter(PoolFairnessPolicy::RoundRobin);
    assert!(first_turn.is_some(), "the first handle must get a turn");
    assert!(second_turn.is_some(), "the second handle must get a turn");
    assert!(
        state
            .take_next_waiter(PoolFairnessPolicy::RoundRobin)
            .is_none(),
        "assertion failed: state .take_next_waiter(PoolFairnessPolicy::RoundRobin) .is_none()"
    );
}

#[test]
fn has_queued_waiter_reports_only_pending_requests() {
    let mut state = TestState::new();
    let _receiver = enqueue_one(&mut state, 3, PoolFairnessPolicy::Fifo, 8);

    assert!(
        state.has_queued_waiter(3),
        "assertion failed: state.has_queued_waiter(3)"
    );
    assert!(
        !state.has_queued_waiter(4),
        "assertion failed: !state.has_queued_waiter(4)"
    );
}

proptest! {
    #![proptest_config(ProptestConfig {
        cases: 64,
        ..ProptestConfig::default()
    })]

    #[test]
    fn generated_waiter_transitions_preserve_admission_bound(
        round_robin in any::<bool>(),
        max_waiters in 1_usize..=8,
        operations in prop::collection::vec((0_u8..5, 0_u8..3), 0..128),
    ) {
        let policy = if round_robin {
            PoolFairnessPolicy::RoundRobin
        } else {
            PoolFairnessPolicy::Fifo
        };
        let waiter_admission = admission(policy, max_waiters);
        let mut state = TestState::new();
        // Receivers are kept so their senders stay open until this test drops
        // them; dropping one models a caller cancelling its acquisition.
        let mut receivers = VecDeque::new();

        for (operation, handle_index) in operations {
            let handle_id = u64::from(handle_index) + 1;
            match operation {
                0 => {
                    let (sender, receiver) = oneshot::channel();
                    match state.enqueue_waiter(handle_id, sender, waiter_admission) {
                        WaiterEnqueueResult::Enqueued => receivers.push_back(receiver),
                        WaiterEnqueueResult::Saturated { rejected } => {
                            // A refusal must occur exactly at the bound, never
                            // above it, and must hand the sender back so the
                            // actor can resolve it with the typed error.
                            prop_assert_eq!(
                                state.waiter_count(),
                                max_waiters,
                                "saturation must happen exactly at the admission bound"
                            );
                            drop(rejected);
                            drop(receiver);
                        }
                    }
                }
                1 => {
                    drop(receivers.pop_front());
                }
                2 => {
                    drop(state.take_next_waiter(policy));
                }
                3 => state.deregister_handle(handle_id),
                4 => state.prune_cancelled_waiters(),
                _ => {}
            }

            prop_assert!(
                state.waiter_count() <= max_waiters,
                "admitted waiters must never exceed the configured bound"
            );
        }
    }
}
