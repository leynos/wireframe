//! Coverage for waiter queue bookkeeping and bounded admission.

use rstest::rstest;

use super::*;
use crate::serializer::BincodeSerializer;

type TestState = SchedulerState<BincodeSerializer, (), ()>;
type TestScheduler = PoolScheduler<BincodeSerializer, (), ()>;

fn admission(fairness_policy: PoolFairnessPolicy, max_waiters: usize) -> WaiterAdmission {
    WaiterAdmission {
        fairness_policy,
        max_waiters,
    }
}

#[test]
fn try_begin_servicing_has_one_owner_until_stopped() {
    let fifo_scheduler = TestScheduler::new(PoolFairnessPolicy::Fifo, 64);
    assert!(fifo_scheduler.try_begin_servicing());
    assert!(!fifo_scheduler.try_begin_servicing());
}

#[test]
fn stop_servicing_allows_another_owner() {
    let fifo_scheduler = TestScheduler::new(PoolFairnessPolicy::Fifo, 64);
    assert!(fifo_scheduler.try_begin_servicing());
    fifo_scheduler.stop_servicing();
    assert!(fifo_scheduler.try_begin_servicing());
}

#[test]
fn restart_if_waiters_only_begins_service_for_queued_work() {
    let scheduler = TestScheduler::new(PoolFairnessPolicy::RoundRobin, 64);
    let handle_id = scheduler.register_handle();
    let (sender, _receiver) = oneshot::channel();
    assert!(!scheduler.restart_if_waiters());
    assert_eq!(
        lock_or_recover(&scheduler.state).enqueue_waiter(
            handle_id,
            sender,
            admission(PoolFairnessPolicy::RoundRobin, 64),
        ),
        WaiterEnqueueResult::Enqueued
    );

    assert!(scheduler.restart_if_waiters());
}

#[test]
fn round_robin_enqueue_rejects_unknown_handles() {
    let mut state = TestState::new();
    let (sender, _receiver) = oneshot::channel();
    let result = state.enqueue_waiter(42, sender, admission(PoolFairnessPolicy::RoundRobin, 64));
    assert_eq!(result, WaiterEnqueueResult::UnregisteredHandle);
    assert!(
        state
            .take_next_waiter(PoolFairnessPolicy::RoundRobin)
            .is_none(),
        "rejected round-robin waiters must not remain queued"
    );
}

#[rstest]
#[case(PoolFairnessPolicy::Fifo)]
#[case(PoolFairnessPolicy::RoundRobin)]
fn enqueue_rejects_waiters_above_the_configured_limit(#[case] policy: PoolFairnessPolicy) {
    let mut state = TestState::new();
    state.register_handle(1);
    state.register_handle(2);
    let mut receivers = Vec::with_capacity(2);
    for handle_id in [1, 2] {
        let (sender, receiver) = oneshot::channel();
        assert_eq!(
            state.enqueue_waiter(handle_id, sender, admission(policy, 2)),
            WaiterEnqueueResult::Enqueued
        );
        receivers.push(receiver);
    }

    let (sender, _receiver) = oneshot::channel();
    assert_eq!(
        state.enqueue_waiter(1, sender, admission(policy, 2)),
        WaiterEnqueueResult::Saturated
    );
    assert_eq!(state.waiter_count(), 2);
}

#[rstest]
#[case(PoolFairnessPolicy::Fifo)]
#[case(PoolFairnessPolicy::RoundRobin)]
fn waiter_count_includes_a_request_being_serviced(#[case] policy: PoolFairnessPolicy) {
    let mut state = TestState::new();
    state.register_handle(1);
    let (sender, _receiver) = oneshot::channel();
    assert_eq!(
        state.enqueue_waiter(1, sender, admission(policy, 1)),
        WaiterEnqueueResult::Enqueued
    );

    let servicing_waiter = state
        .take_next_waiter(policy)
        .expect("enqueued waiter should be selected");
    assert_eq!(state.waiter_count(), 1);

    state.finish_servicing_waiter();
    assert_eq!(state.waiter_count(), 0);
    drop(servicing_waiter);
}

#[rstest]
#[case(PoolFairnessPolicy::Fifo)]
#[case(PoolFairnessPolicy::RoundRobin)]
fn enqueue_prunes_cancelled_waiters_before_counting(#[case] policy: PoolFairnessPolicy) {
    let mut state = TestState::new();
    state.register_handle(1);
    state.register_handle(2);
    let (cancelled_sender, cancelled_receiver) = oneshot::channel();
    let (live_sender, _live_receiver) = oneshot::channel();
    assert_eq!(
        state.enqueue_waiter(1, cancelled_sender, admission(policy, 2)),
        WaiterEnqueueResult::Enqueued
    );
    assert_eq!(
        state.enqueue_waiter(2, live_sender, admission(policy, 2)),
        WaiterEnqueueResult::Enqueued
    );
    drop(cancelled_receiver);

    let (sender, _receiver) = oneshot::channel();
    assert_eq!(
        state.enqueue_waiter(1, sender, admission(policy, 2)),
        WaiterEnqueueResult::Enqueued
    );
    assert_eq!(state.waiter_count(), 2);
}

#[test]
fn prune_cancelled_waiters_cleans_fifo_and_handle_queues() {
    let mut state = TestState::new();
    state.register_handle(1);
    let (fifo_sender, fifo_receiver) = oneshot::channel();
    let (handle_sender, handle_receiver) = oneshot::channel();
    state.fifo_queue.push_back((1, fifo_sender));
    state
        .handle_waiters
        .get_mut(&1)
        .expect("registered handle queue")
        .push_back(handle_sender);
    drop((fifo_receiver, handle_receiver));

    state.prune_cancelled_waiters();

    assert_eq!(state.waiter_count(), 0);
}

#[test]
fn deregister_handle_purges_fifo_entries_for_that_handle() {
    let mut state = TestState::new();
    state.register_handle(1);
    state.register_handle(2);
    let (removed_sender, _removed_receiver) = oneshot::channel();
    let (kept_sender, _kept_receiver) = oneshot::channel();
    assert_eq!(
        state.enqueue_waiter(1, removed_sender, admission(PoolFairnessPolicy::Fifo, 64)),
        WaiterEnqueueResult::Enqueued
    );
    assert_eq!(
        state.enqueue_waiter(2, kept_sender, admission(PoolFairnessPolicy::Fifo, 64)),
        WaiterEnqueueResult::Enqueued
    );
    state.deregister_handle(1);
    let next_waiter = state.take_next_waiter(PoolFairnessPolicy::Fifo);
    assert!(
        next_waiter.is_some(),
        "remaining registered waiter should still be queued"
    );
    assert!(
        state.take_next_waiter(PoolFairnessPolicy::Fifo).is_none(),
        "deregistered handle entries must be removed eagerly"
    );
}
