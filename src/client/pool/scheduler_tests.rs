//! Coverage for waiter queue bookkeeping across fairness policies.

use std::{
    sync::{Arc, Mutex},
    thread,
};

use googletest::{gtest, prelude::*};
use tokio::sync::oneshot;
use tracing::Level;
use wireframe_testing::ObservabilityHandle;

use super::{super::test_support::CaptureWriter, *};
use crate::serializer::BincodeSerializer;

type TestScheduler = PoolScheduler<BincodeSerializer, (), ()>;
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

#[gtest]
fn waiters_remain_ordered_after_scheduler_state_poison_recovery() {
    for policy in [PoolFairnessPolicy::Fifo, PoolFairnessPolicy::RoundRobin] {
        let scheduler = Arc::new(TestScheduler::new(policy));
        let first_handle = scheduler.register_handle();
        let second_handle = scheduler.register_handle();
        let (first_sender, mut first_receiver) = oneshot::channel();
        let (second_sender, mut second_receiver) = oneshot::channel();
        let (third_sender, mut third_receiver) = oneshot::channel();

        {
            let mut state = lock_or_recover(&scheduler.state);
            expect_that!(
                state.enqueue_waiter(first_handle, first_sender, policy),
                eq(true)
            );
            expect_that!(
                state.enqueue_waiter(second_handle, second_sender, policy),
                eq(true)
            );
            expect_that!(
                state.enqueue_waiter(first_handle, third_sender, policy),
                eq(true)
            );
            expect_that!(state.has_waiters(), eq(true));
        }

        poison_scheduler_state(&scheduler);
        let recovered_handle = recover_scheduler_state(&scheduler);

        let mut state = scheduler
            .state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        expect_that!(
            state.handle_waiters.contains_key(&recovered_handle),
            eq(true)
        );
        expect_that!(state.has_waiters(), eq(true));

        drop_next_waiter(&mut state, policy, "first waiter");
        assert_receiver_closed(&mut first_receiver);
        drop_next_waiter(&mut state, policy, "second waiter");
        assert_receiver_closed(&mut second_receiver);
        drop_next_waiter(&mut state, policy, "third waiter");
        assert_receiver_closed(&mut third_receiver);
        expect_that!(state.has_waiters(), eq(false));
        assert!(
            state.take_next_waiter(policy).is_none(),
            "all waiters must be served exactly once"
        );
    }
}

fn drop_next_waiter(state: &mut TestState, policy: PoolFairnessPolicy, waiter_name: &str) {
    let Some(waiter) = state.take_next_waiter(policy) else {
        panic!("{waiter_name} should remain queued");
    };
    drop(waiter);
}

fn assert_receiver_closed<T>(receiver: &mut oneshot::Receiver<T>) {
    assert!(
        matches!(
            receiver.try_recv(),
            Err(oneshot::error::TryRecvError::Closed)
        ),
        "served waiter must not be lost or served twice"
    );
}

fn poison_scheduler_state(scheduler: &Arc<TestScheduler>) {
    let poisoned_scheduler = Arc::clone(scheduler);
    let join_result = thread::spawn(move || {
        let _state = lock_or_recover(&poisoned_scheduler.state);
        panic!("poison scheduler state for recovery coverage");
    })
    .join();

    expect_that!(join_result, err(anything()));
    expect_that!(scheduler.state.is_poisoned(), eq(true));
}

fn recover_scheduler_state(scheduler: &Arc<TestScheduler>) -> u64 {
    let captured_logs = Arc::new(Mutex::new(Vec::new()));
    let subscriber = tracing_subscriber::fmt()
        .with_ansi(false)
        .without_time()
        .with_max_level(Level::WARN)
        .with_writer(CaptureWriter::new(Arc::clone(&captured_logs)))
        .finish();
    let mut observability = ObservabilityHandle::new();
    let recovered_handle = metrics::with_local_recorder(observability.recorder(), || {
        tracing::subscriber::with_default(subscriber, || scheduler.register_handle())
    });
    observability.snapshot();
    let log_bytes = lock_or_recover(&captured_logs).clone();
    let logs = String::from_utf8_lossy(&log_bytes);

    expect_that!(
        logs.as_ref(),
        contains_substring("recovering poisoned client pool bookkeeping lock")
    );
    expect_that!(
        observability.counter_without_labels(crate::metrics::POOL_BOOKKEEPING_POISON_RECOVERIES),
        eq(1)
    );

    recovered_handle
}
