//! Coverage for waiter queue bookkeeping across fairness policies.

use super::*;
use crate::serializer::BincodeSerializer;

type TestState = SchedulerState<BincodeSerializer, (), ()>;

#[test]
fn round_robin_enqueue_rejects_unknown_handles() {
    let mut state = TestState::new();
    let (sender, _receiver) = oneshot::channel();

    let was_enqueued = state.enqueue_waiter(42, sender, PoolFairnessPolicy::RoundRobin);

    assert!(
        !was_enqueued,
        "unknown round-robin handles must be rejected"
    );
    assert!(
        state
            .take_next_waiter(PoolFairnessPolicy::RoundRobin)
            .is_none(),
        "rejected round-robin waiters must not remain queued"
    );
}

#[test]
fn deregister_handle_purges_fifo_entries_for_that_handle() {
    let mut state = TestState::new();
    state.register_handle(1);
    state.register_handle(2);

    let (removed_sender, _removed_receiver) = oneshot::channel();
    let (kept_sender, _kept_receiver) = oneshot::channel();

    assert!(state.enqueue_waiter(1, removed_sender, PoolFairnessPolicy::Fifo));
    assert!(state.enqueue_waiter(2, kept_sender, PoolFairnessPolicy::Fifo));

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
