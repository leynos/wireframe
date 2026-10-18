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

const MAX_TEST_WAITERS: usize = 64;

fn test_admission(fairness_policy: PoolFairnessPolicy) -> WaiterAdmission {
    WaiterAdmission {
        fairness_policy,
        max_waiters: MAX_TEST_WAITERS,
    }
}

#[gtest]
fn waiters_remain_ordered_after_scheduler_state_poison_recovery() {
    for policy in [PoolFairnessPolicy::Fifo, PoolFairnessPolicy::RoundRobin] {
        let scheduler = Arc::new(TestScheduler::new(policy, MAX_TEST_WAITERS));
        let first_handle = scheduler.register_handle();
        let second_handle = scheduler.register_handle();
        let (first_sender, mut first_receiver) = oneshot::channel();
        let (second_sender, mut second_receiver) = oneshot::channel();
        let (third_sender, mut third_receiver) = oneshot::channel();

        {
            let mut state = lock_or_recover(&scheduler.state);
            assert_eq!(
                state.enqueue_waiter(first_handle, first_sender, test_admission(policy)),
                WaiterEnqueueResult::Enqueued
            );
            assert_eq!(
                state.enqueue_waiter(first_handle, second_sender, test_admission(policy)),
                WaiterEnqueueResult::Enqueued
            );
            expect_that!(state.waiter_count(), eq(2));
            expect_that!(state.has_waiters(), eq(true));
        }

        poison_scheduler_state_during_enqueue(&scheduler, second_handle, third_sender, policy);
        let recovered_handle = recover_scheduler_state(&scheduler);

        let mut state = lock_or_recover(&scheduler.state);
        expect_that!(
            state.handle_waiters.contains_key(&recovered_handle),
            eq(true)
        );
        expect_that!(state.waiter_count(), eq(3));
        expect_that!(state.has_waiters(), eq(true));

        match policy {
            PoolFairnessPolicy::Fifo => {
                assert_next_waiter_served(&mut state, policy, &mut first_receiver, 2);
                assert_next_waiter_served(&mut state, policy, &mut second_receiver, 1);
                assert_next_waiter_served(&mut state, policy, &mut third_receiver, 0);
            }
            PoolFairnessPolicy::RoundRobin => {
                assert_next_waiter_served(&mut state, policy, &mut first_receiver, 2);
                assert_next_waiter_served(&mut state, policy, &mut third_receiver, 1);
                assert_next_waiter_served(&mut state, policy, &mut second_receiver, 0);
            }
        }
        expect_that!(state.has_waiters(), eq(false));
        assert!(
            state.take_next_waiter(policy).is_none(),
            "all waiters must be served exactly once"
        );
    }
}

fn assert_next_waiter_served<T>(
    state: &mut TestState,
    policy: PoolFairnessPolicy,
    receiver: &mut oneshot::Receiver<T>,
    remaining_waiters: usize,
) {
    drop_next_waiter(state, policy, "expected waiter");
    assert_receiver_closed(receiver);
    expect_that!(state.waiter_count(), eq(remaining_waiters));
    expect_that!(state.has_waiters(), eq(remaining_waiters > 0));
}

fn drop_next_waiter(state: &mut TestState, policy: PoolFairnessPolicy, waiter_name: &str) {
    let Some(waiter) = state.take_next_waiter(policy) else {
        panic!("{waiter_name} should remain queued");
    };
    drop(waiter);
    state.finish_servicing_waiter();
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

fn poison_scheduler_state_during_enqueue(
    scheduler: &Arc<TestScheduler>,
    handle_id: u64,
    sender: WaiterSender<BincodeSerializer, (), ()>,
    policy: PoolFairnessPolicy,
) {
    let poisoned_scheduler = Arc::clone(scheduler);
    let join_result = thread::spawn(move || {
        let mut state = lock_or_recover(&poisoned_scheduler.state);
        assert_eq!(
            state.enqueue_waiter(handle_id, sender, test_admission(policy)),
            WaiterEnqueueResult::Enqueued
        );
        panic!("panic after waiter enqueue while holding scheduler state");
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
