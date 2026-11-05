//! Deterministic coverage for the scheduler's synchronous decision handlers.
//!
//! These tests drive [`SchedulerTask`] without spawning it. Every fairness,
//! admission, and termination decision is a synchronous method on the task, so
//! the expected ordering can be asserted exactly instead of sampled from a
//! running event loop.

use std::time::Duration;

use tokio::sync::oneshot;

use super::*;
use crate::{
    client::pool::{
        PoolTaskChurnObserver,
        churn::PoolTaskChurnCounts,
        scheduler::handle::SchedulerLifecycle,
        test_support::test_pool_slot,
    },
    serializer::BincodeSerializer,
};

type TestCore = PoolCore<BincodeSerializer, (), ()>;
type TestTask = SchedulerTask<BincodeSerializer, (), ()>;
type TestLease = PooledClientLease<BincodeSerializer, (), ()>;
type TestReply = oneshot::Receiver<Result<TestLease, ClientError>>;
type TestHandleId = u64;

/// One task, its core, and the mailbox sender that keeps the receiver open.
struct Harness {
    core: Arc<TestCore>,
    status: Arc<SchedulerStatus>,
    churn: Arc<PoolTaskChurnCounts>,
    sender: mpsc::UnboundedSender<SchedulerCommand<BincodeSerializer, (), ()>>,
    task: TestTask,
    /// Lease holding the pool's capacity while a test wants callers to queue.
    held: Option<TestLease>,
    /// Time driver the slots registered with when they were built.
    ///
    /// Declared last so it outlives every slot that borrowed its reactor.
    _runtime: tokio::runtime::Runtime,
}

impl Harness {
    /// Build a task over `slot_count` fixed slots.
    ///
    /// A slot's backing `bb8` pool builds a time interval eagerly, so slot
    /// construction needs a reactor even though these tests never await. The
    /// runtime stays alive for the harness's lifetime rather than being
    /// dropped straight after construction, so nothing outlives its driver.
    fn with_slots(slot_count: usize, policy: PoolFairnessPolicy, max_waiters: usize) -> Self {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("build a current-thread runtime");
        let slots = {
            let _guard = runtime.enter();
            (0..slot_count)
                .map(|_| test_pool_slot(1, Duration::from_secs(30)))
                .collect::<Vec<_>>()
        };
        let core = Arc::new(PoolCore::new(slots.into_boxed_slice()));
        let churn = Arc::new(PoolTaskChurnCounts::default());
        let status = Arc::new(SchedulerStatus::new(Arc::clone(&churn)));
        let (sender, receiver) = mpsc::unbounded_channel();
        let task = SchedulerTask::new(
            Arc::clone(&core),
            Arc::clone(&status),
            receiver,
            WaiterAdmission {
                fairness_policy: policy,
                max_waiters,
            },
        );
        Self {
            core,
            status,
            churn,
            sender,
            task,
            held: None,
            _runtime: runtime,
        }
    }

    /// Submit one acquisition directly to the task's command handler.
    fn acquire(&mut self, handle_id: TestHandleId) -> TestReply {
        let (reply, receiver) = oneshot::channel();
        self.task
            .handle_command(&self.core, SchedulerCommand::Acquire { handle_id, reply });
        receiver
    }

    /// Take the first slot's only permit so later acquisitions must queue.
    fn hold_only_permit(&mut self) {
        let slot = self.core.slot(0).expect("the only slot must exist");
        let permit = slot
            .try_acquire_permit()
            .expect("a fresh slot must grant an immediate permit");
        self.held = Some(
            PooledClientLease::new(Arc::clone(&self.core), 0, permit)
                .expect("slot zero must resolve in the core"),
        );
    }

    /// Release the held permit, letting a sweep grant it to a waiter.
    fn release_permit(&mut self) { drop(self.held.take()); }
}

/// A receiver that has been granted a lease by the sweep.
fn take_grant(reply: &mut TestReply) -> TestLease {
    match reply.try_recv() {
        Ok(Ok(lease)) => lease,
        Ok(Err(error)) => panic!("expected a lease, got a refusal: {error}"),
        Err(error) => panic!("expected a lease, got {error:?}"),
    }
}

/// A receiver that was refused, returning the typed error.
fn take_refusal(reply: &mut TestReply) -> ClientError {
    match reply.try_recv() {
        Ok(Err(error)) => error,
        Ok(Ok(_lease)) => panic!("expected a refusal, got a lease"),
        Err(error) => panic!("expected a refusal, got {error:?}"),
    }
}

#[test]
fn immediate_capacity_is_granted_without_queueing() {
    let mut harness = Harness::with_slots(1, PoolFairnessPolicy::Fifo, 8);

    let mut reply = harness.acquire(1);
    let lease = take_grant(&mut reply);

    assert_eq!(harness.task.state.waiter_count(), 0, "no waiter remains");
    drop(lease);
}

#[test]
fn fifo_grants_queued_waiters_in_arrival_order() {
    let mut harness = Harness::with_slots(1, PoolFairnessPolicy::Fifo, 8);
    harness.hold_only_permit();
    let mut first = harness.acquire(1);
    let mut second = harness.acquire(2);
    assert_eq!(harness.task.state.waiter_count(), 2, "two waiters queue");

    harness.release_permit();
    harness.task.grant_sweep(&harness.core);

    let first_lease = take_grant(&mut first);
    assert!(second.try_recv().is_err(), "second arrival must wait");

    drop(first_lease);
    harness.task.grant_sweep(&harness.core);
    drop(take_grant(&mut second));
}

#[test]
fn round_robin_gives_each_handle_one_turn_before_repeating() {
    let mut harness = Harness::with_slots(1, PoolFairnessPolicy::RoundRobin, 8);
    harness.hold_only_permit();
    let mut waiters = vec![
        (1, harness.acquire(1)),
        (2, harness.acquire(2)),
        (3, harness.acquire(3)),
        (1, harness.acquire(1)),
        (2, harness.acquire(2)),
        (3, harness.acquire(3)),
    ];

    let mut granted = Vec::new();
    while let Some((handle_id, lease)) = grant_next(&mut harness, &mut waiters) {
        granted.push(handle_id);
        harness.held = Some(lease);
    }

    assert_eq!(granted, vec![1, 2, 3, 1, 2, 3], "each handle one turn");
}

/// Release one permit and return whichever waiter received it.
fn grant_next(
    harness: &mut Harness,
    waiters: &mut [(TestHandleId, TestReply)],
) -> Option<(TestHandleId, TestLease)> {
    harness.release_permit();
    harness.task.grant_sweep(&harness.core);
    for (handle_id, reply) in waiters.iter_mut() {
        if let Ok(Ok(lease)) = reply.try_recv() {
            return Some((*handle_id, lease));
        }
    }
    None
}

#[test]
fn a_waiter_above_the_admission_bound_is_refused_typed() {
    let mut harness = Harness::with_slots(1, PoolFairnessPolicy::Fifo, 1);
    harness.hold_only_permit();
    let _queued = harness.acquire(1);

    let mut rejected = harness.acquire(2);

    let error = take_refusal(&mut rejected);
    assert!(
        matches!(error, ClientError::PoolSaturated { limit: 1 }),
        "waiter bound exceeded"
    );
}

#[test]
fn a_deregistered_handle_is_never_granted() {
    let mut harness = Harness::with_slots(1, PoolFairnessPolicy::Fifo, 8);
    harness.hold_only_permit();
    let mut abandoned = harness.acquire(1);
    harness.task.handle_command(
        &harness.core,
        SchedulerCommand::DeregisterHandle { handle_id: 1 },
    );

    harness.release_permit();
    harness.task.grant_sweep(&harness.core);

    assert!(
        matches!(
            abandoned.try_recv(),
            Err(oneshot::error::TryRecvError::Closed)
        ),
        "deregistered handle is never granted"
    );
    assert_eq!(harness.task.state.waiter_count(), 0, "no stranded waiter");
}

#[test]
fn a_permit_race_error_fails_the_scheduler_and_refuses_waiters() {
    let mut harness = Harness::with_slots(1, PoolFairnessPolicy::Fifo, 8);
    harness.hold_only_permit();
    let mut queued = harness.acquire(1);

    harness
        .task
        .handle_permit_outcome(&harness.core, Some(Err(ClientError::scheduler_failed())));

    assert_eq!(
        harness.status.lifecycle(),
        SchedulerLifecycle::Failed,
        "scheduler marked failed"
    );
    let error = take_refusal(&mut queued);
    assert!(
        matches!(error, ClientError::SchedulerFailed),
        "waiter fails"
    );
}

#[test]
fn shutdown_drains_queued_and_buffered_acquisitions() {
    let mut harness = Harness::with_slots(1, PoolFairnessPolicy::Fifo, 8);
    harness.hold_only_permit();
    let mut queued = harness.acquire(1);
    let (reply, mut buffered) = oneshot::channel();
    harness
        .sender
        .send(SchedulerCommand::Acquire {
            handle_id: 2,
            reply,
        })
        .expect("the mailbox must accept a buffered command");

    harness.task.drain_for_shutdown();

    assert_eq!(
        harness.status.lifecycle(),
        SchedulerLifecycle::Closed,
        "shutdown closes scheduler"
    );
    assert!(
        matches!(take_refusal(&mut queued), ClientError::PoolClosed),
        "queued waiter refused"
    );
    assert!(
        matches!(take_refusal(&mut buffered), ClientError::PoolClosed),
        "buffered acquisition refused at shutdown"
    );
    assert_eq!(harness.task.state.waiter_count(), 0, "no waiters survive");
}

#[test]
fn an_acquisition_after_closure_is_refused_without_queueing() {
    let mut harness = Harness::with_slots(1, PoolFairnessPolicy::Fifo, 8);
    harness.status.mark_closed();

    let mut refused = harness.acquire(1);

    assert!(
        matches!(take_refusal(&mut refused), ClientError::PoolClosed),
        "refused after closure"
    );
    assert_eq!(harness.task.state.waiter_count(), 0, "nothing is queued");
}

#[test]
fn capacity_wait_counts_only_requests_that_queued() {
    let mut harness = Harness::with_slots(1, PoolFairnessPolicy::Fifo, 8);
    let observer = PoolTaskChurnObserver::new(Arc::clone(&harness.churn));

    let mut immediate = harness.acquire(1);
    harness.held = Some(take_grant(&mut immediate));
    assert_eq!(observer.snapshot().capacity_waits, 0, "no capacity wait");

    let _queued = harness.acquire(2);
    assert_eq!(observer.snapshot().capacity_waits, 1, "one capacity wait");
}

// `actor_tests` is loaded through `#[path]`, which makes child modules resolve
// beside it rather than in a directory named after it, so the submodule states
// its own path explicitly.
#[cfg(all(feature = "metrics", feature = "pool", not(loom)))]
#[path = "actor_tests/metrics.rs"]
mod metrics;
