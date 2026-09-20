//! Fair lease scheduling for pooled client handles.

use std::{
    collections::{HashMap, VecDeque},
    sync::{
        Arc,
        Mutex,
        atomic::{AtomicBool, AtomicU64, Ordering},
    },
};

use tokio::sync::oneshot;

use super::{
    client_pool::ClientPoolInner,
    lease::PooledClientLease,
    policy::PoolFairnessPolicy,
    sync::lock_or_recover,
};
use crate::{client::ClientError, serializer::Serializer};

/// One-shot completion channel for a queued logical-session acquisition.
type WaiterSender<S, P, C> = oneshot::Sender<Result<PooledClientLease<S, P, C>, ClientError>>;
/// Policy-specific waiter queues protected by the scheduler mutex.
struct SchedulerState<S, P, C>
where
    S: Serializer + Clone + Send + Sync + 'static,
    P: bincode::Encode + Clone + Send + Sync + 'static,
    C: Send + 'static,
{
    /// Global arrival order used by FIFO admission.
    fifo_queue: VecDeque<(u64, WaiterSender<S, P, C>)>,
    /// Rotating handle identities used to provide per-session turns.
    round_robin_order: VecDeque<u64>,
    /// Per-handle queues preserving request order within a session.
    handle_waiters: HashMap<u64, VecDeque<WaiterSender<S, P, C>>>,
}

impl<S, P, C> SchedulerState<S, P, C>
where
    S: Serializer + Clone + Send + Sync + 'static,
    P: bincode::Encode + Clone + Send + Sync + 'static,
    C: Send + 'static,
{
    /// Start with empty queues and no registered logical sessions.
    fn new() -> Self {
        Self {
            fifo_queue: VecDeque::new(),
            round_robin_order: VecDeque::new(),
            handle_waiters: HashMap::new(),
        }
    }

    /// Add a handle to both identity tracking and round-robin rotation.
    fn register_handle(&mut self, handle_id: u64) {
        self.handle_waiters.insert(handle_id, VecDeque::new());
        self.round_robin_order.push_back(handle_id);
    }

    /// Remove a handle and all queued requests so dropped callers cannot win capacity.
    fn deregister_handle(&mut self, handle_id: u64) {
        self.handle_waiters.remove(&handle_id);
        self.round_robin_order
            .retain(|queued_id| *queued_id != handle_id);
        self.fifo_queue
            .retain(|(queued_id, _)| *queued_id != handle_id);
    }

    /// Queue ownership requires registered round-robin handles; FIFO tolerates stale owners.
    fn enqueue_waiter(
        &mut self,
        handle_id: u64,
        sender: WaiterSender<S, P, C>,
        policy: PoolFairnessPolicy,
    ) -> bool {
        match policy {
            PoolFairnessPolicy::Fifo => {
                self.fifo_queue.push_back((handle_id, sender));
                true
            }
            PoolFairnessPolicy::RoundRobin => {
                if let Some(queue) = self.handle_waiters.get_mut(&handle_id) {
                    queue.push_back(sender);
                    true
                } else {
                    false
                }
            }
        }
    }

    /// Report whether any policy queue still contains a request.
    fn has_waiters(&self) -> bool {
        !self.fifo_queue.is_empty() || self.handle_waiters.values().any(|queue| !queue.is_empty())
    }

    /// Select one waiter according to the configured ordering policy.
    fn take_next_waiter(&mut self, policy: PoolFairnessPolicy) -> Option<WaiterSender<S, P, C>> {
        match policy {
            PoolFairnessPolicy::RoundRobin => self.take_next_round_robin_waiter(),
            PoolFairnessPolicy::Fifo => self.take_next_fifo_waiter(),
        }
    }

    /// Pop FIFO arrivals, skipping requests from handles already dropped.
    fn take_next_fifo_waiter(&mut self) -> Option<WaiterSender<S, P, C>> {
        while let Some((handle_id, sender)) = self.fifo_queue.pop_front() {
            if self.handle_waiters.contains_key(&handle_id) {
                return Some(sender);
            }
        }
        None
    }

    /// Visit each registered handle once and pop its oldest pending request.
    fn take_next_round_robin_waiter(&mut self) -> Option<WaiterSender<S, P, C>> {
        let len = self.round_robin_order.len();
        for _ in 0..len {
            let handle_id = self.round_robin_order.pop_front()?;
            self.round_robin_order.push_back(handle_id);
            if let Some(queue) = self.handle_waiters.get_mut(&handle_id)
                && let Some(sender) = queue.pop_front()
            {
                return Some(sender);
            }
        }
        None
    }
}

/// Shared fairness scheduler used by pooled handles.
pub(crate) struct PoolScheduler<S, P, C>
where
    S: Serializer + Clone + Send + Sync + 'static,
    P: bincode::Encode + Clone + Send + Sync + 'static,
    C: Send + 'static,
{
    /// Policy chosen for all blocked acquisitions in this pool.
    fairness_policy: PoolFairnessPolicy,
    /// Monotonic source of logical-session identities.
    next_handle_id: AtomicU64,
    /// Single-service guard preventing duplicate scheduler tasks.
    is_servicing: AtomicBool,
    /// Mutex-protected queues shared by acquisition and release paths.
    state: Mutex<SchedulerState<S, P, C>>,
}

impl<S, P, C> PoolScheduler<S, P, C>
where
    S: Serializer + Clone + Send + Sync + 'static,
    P: bincode::Encode + Clone + Send + Sync + 'static,
    C: Send + 'static,
{
    /// Create a scheduler with no active handles or waiters.
    pub(crate) fn new(fairness_policy: PoolFairnessPolicy) -> Self {
        Self {
            fairness_policy,
            next_handle_id: AtomicU64::new(1),
            is_servicing: AtomicBool::new(false),
            state: Mutex::new(SchedulerState::new()),
        }
    }

    /// Allocate and register a stable identity for one logical session.
    pub(crate) fn register_handle(&self) -> u64 {
        let handle_id = self.next_handle_id.fetch_add(1, Ordering::Relaxed);
        lock_or_recover(&self.state).register_handle(handle_id);
        handle_id
    }

    /// Remove a session and cancel its queued requests by dropping senders.
    pub(crate) fn deregister_handle(&self, handle_id: u64) {
        lock_or_recover(&self.state).deregister_handle(handle_id);
    }

    /// Queue an acquisition, taking an immediate lease when capacity permits.
    pub(crate) async fn acquire_for_handle(
        self: &Arc<Self>,
        inner: Arc<ClientPoolInner<S, P, C>>,
        handle_id: u64,
    ) -> Result<PooledClientLease<S, P, C>, ClientError> {
        if inner.is_shutdown() {
            return Err(ClientError::disconnected());
        }

        let (sender, receiver) = oneshot::channel();
        if !lock_or_recover(&self.state).enqueue_waiter(handle_id, sender, self.fairness_policy) {
            tracing::warn!(
                handle_id,
                fairness_policy = ?self.fairness_policy,
                "pooled handle enqueue attempted for unregistered handle"
            );
            return Err(ClientError::disconnected());
        }

        if let Some(lease) = inner.try_acquire_immediately() {
            return self.resolve_immediate_lease(lease, receiver).await;
        }

        self.kick(inner);
        receiver.await.map_err(|_| ClientError::disconnected())?
    }

    /// Complete every queued waiter with a disconnected error.
    pub(crate) fn notify_shutdown(&self) {
        while let Some(waiter) = self.next_waiter() {
            let _ = waiter.send(Err(ClientError::disconnected()));
        }
    }

    /// Restart service when a released lease may satisfy a waiter.
    pub(crate) fn notify_capacity_available(
        self: &Arc<Self>,
        inner: Arc<ClientPoolInner<S, P, C>>,
    ) {
        self.kick(inner);
    }

    /// Spawn at most one worker to drain waiters as capacity appears.
    fn kick(self: &Arc<Self>, inner: Arc<ClientPoolInner<S, P, C>>) {
        if self.try_begin_servicing() {
            let scheduler = Arc::clone(self);
            tokio::spawn(async move {
                scheduler.service_waiters(inner).await;
            });
        }
    }

    /// Re-arm the worker if a race left requests after it stopped.
    fn restart_if_waiters(&self) -> bool {
        if !lock_or_recover(&self.state).has_waiters() {
            return false;
        }

        self.try_begin_servicing()
    }

    /// Dequeue a waiter, closing the worker only after queues are empty.
    fn take_next_waiter_or_stop(&self) -> Option<WaiterSender<S, P, C>> {
        loop {
            if let Some(sender) = self.next_waiter() {
                return Some(sender);
            }

            self.stop_servicing();
            if !self.restart_if_waiters() {
                return None;
            }
        }
    }

    /// Select the next waiter through the pool's configured fairness policy.
    fn next_waiter(&self) -> Option<WaiterSender<S, P, C>> {
        lock_or_recover(&self.state).take_next_waiter(self.fairness_policy)
    }

    /// Hand an immediate lease to the selected waiter, or retain it for this caller.
    async fn resolve_immediate_lease(
        &self,
        lease: PooledClientLease<S, P, C>,
        receiver: oneshot::Receiver<Result<PooledClientLease<S, P, C>, ClientError>>,
    ) -> Result<PooledClientLease<S, P, C>, ClientError> {
        let Some(waiter) = self.next_waiter() else {
            drop(receiver);
            return Ok(lease);
        };

        if waiter.send(Ok(lease)).is_err() {
            drop(receiver);
            return Err(ClientError::disconnected());
        }

        receiver.await.map_err(|_| ClientError::disconnected())?
    }

    /// Claim the idle-to-servicing hand-off so one worker owns the enqueue race.
    fn try_begin_servicing(&self) -> bool {
        self.is_servicing
            .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
            .is_ok()
    }

    /// Publish servicing-to-idle before rechecking waiters that raced with emptiness.
    fn stop_servicing(&self) { self.is_servicing.store(false, Ordering::Release); }

    /// Serially service waiters so fairness order is deterministic.
    async fn service_waiters(self: Arc<Self>, inner: Arc<ClientPoolInner<S, P, C>>) {
        while let Some(sender) = self.take_next_waiter_or_stop() {
            self.service_one_waiter(sender, Arc::clone(&inner)).await;
        }
    }

    /// Race slot capacity against shutdown for one selected waiter.
    #[expect(
        clippy::integer_division_remainder_used,
        reason = "tokio::select! macro internally uses % for random branch selection"
    )]
    async fn service_one_waiter(
        &self,
        sender: WaiterSender<S, P, C>,
        inner: Arc<ClientPoolInner<S, P, C>>,
    ) {
        let result = tokio::select! {
            permit_result = inner.acquire_slot_permit() => {
                permit_result.map(|(slot, permit)| {
                    PooledClientLease::new(slot, permit, Some(Arc::clone(&inner)))
                })
            }
            () = inner.shutdown_notified() => {
                let _ = sender.send(Err(ClientError::disconnected()));
                return;
            }
        };

        if let Err(send_result) = sender.send(result)
            && let Ok(lease) = send_result
        {
            drop(lease);
        }
    }
}

#[cfg(test)]
mod tests {
    //! Coverage for waiter queue bookkeeping across fairness policies.

    use rstest::{fixture, rstest};

    use super::*;
    use crate::serializer::BincodeSerializer;

    type TestState = SchedulerState<BincodeSerializer, (), ()>;
    type TestScheduler = PoolScheduler<BincodeSerializer, (), ()>;
    #[fixture]
    fn fifo_scheduler() -> TestScheduler {
        let fairness_policy = PoolFairnessPolicy::Fifo;
        TestScheduler::new(fairness_policy)
    }
    #[rstest]
    fn try_begin_servicing_has_one_owner_until_stopped(fifo_scheduler: TestScheduler) {
        assert!(fifo_scheduler.try_begin_servicing());
        assert!(!fifo_scheduler.try_begin_servicing());
    }
    #[rstest]
    fn stop_servicing_allows_another_owner(fifo_scheduler: TestScheduler) {
        assert!(fifo_scheduler.try_begin_servicing());
        fifo_scheduler.stop_servicing();
        assert!(fifo_scheduler.try_begin_servicing());
    }
    #[test]
    fn restart_if_waiters_only_begins_service_for_queued_work() {
        let scheduler = TestScheduler::new(PoolFairnessPolicy::RoundRobin);
        let handle_id = scheduler.register_handle();
        let (sender, _receiver) = oneshot::channel();
        assert!(!scheduler.restart_if_waiters());
        assert!(lock_or_recover(&scheduler.state).enqueue_waiter(
            handle_id,
            sender,
            PoolFairnessPolicy::RoundRobin,
        ));

        assert!(scheduler.restart_if_waiters());
    }
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
}
