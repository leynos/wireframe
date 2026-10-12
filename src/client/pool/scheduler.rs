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
/// Result of admitting one acquisition to the scheduler's waiter queues.
#[derive(Debug, PartialEq, Eq)]
enum WaiterEnqueueResult {
    /// The request was queued and may take an immediate lease or wait.
    Enqueued,
    /// Round-robin admission requires a live logical handle.
    UnregisteredHandle,
    /// The configured number of live waiters is already queued.
    Saturated,
}
/// Fairness and capacity settings applied to one waiter admission attempt.
#[derive(Clone, Copy)]
struct WaiterAdmission {
    /// Queue ordering policy for this pool.
    fairness_policy: PoolFairnessPolicy,
    /// Maximum live waiter count accepted by the pool.
    max_waiters: usize,
}
/// Policy-specific waiter queues protected by the scheduler mutex.
struct SchedulerState<S, P, C>
where
    S: Serializer + Clone + Send + Sync + 'static,
    P: bincode::Encode + Clone + Send + Sync + 'static,
    C: Send + 'static,
{
    /// Global arrival order used by FIFO admission.
    fifo_queue: VecDeque<(u64, WaiterSender<S, P, C>)>,
    /// Requests removed from a queue while the worker waits for slot capacity.
    servicing_waiters: usize,
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
            servicing_waiters: 0,
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

    /// Remove abandoned acquisitions from every policy queue before admission.
    fn prune_cancelled_waiters(&mut self) {
        self.fifo_queue.retain(|(_, sender)| !sender.is_closed());
        for queue in self.handle_waiters.values_mut() {
            queue.retain(|sender| !sender.is_closed());
        }
    }

    /// Count live requests across queues and the worker's capacity wait.
    fn waiter_count(&self) -> usize {
        self.servicing_waiters
            + self.fifo_queue.len()
            + self
                .handle_waiters
                .values()
                .map(VecDeque::len)
                .sum::<usize>()
    }

    /// Queue ownership requires registered round-robin handles; FIFO tolerates stale owners.
    fn enqueue_waiter(
        &mut self,
        handle_id: u64,
        sender: WaiterSender<S, P, C>,
        admission: WaiterAdmission,
    ) -> WaiterEnqueueResult {
        let WaiterAdmission {
            fairness_policy,
            max_waiters,
        } = admission;
        self.prune_cancelled_waiters();
        if fairness_policy == PoolFairnessPolicy::RoundRobin
            && !self.handle_waiters.contains_key(&handle_id)
        {
            return WaiterEnqueueResult::UnregisteredHandle;
        }
        if self.waiter_count() >= max_waiters {
            return WaiterEnqueueResult::Saturated;
        }

        match fairness_policy {
            PoolFairnessPolicy::Fifo => {
                self.fifo_queue.push_back((handle_id, sender));
                WaiterEnqueueResult::Enqueued
            }
            PoolFairnessPolicy::RoundRobin => match self.handle_waiters.get_mut(&handle_id) {
                Some(queue) => {
                    queue.push_back(sender);
                    WaiterEnqueueResult::Enqueued
                }
                None => WaiterEnqueueResult::UnregisteredHandle,
            },
        }
    }

    /// Report whether any policy queue still contains a request.
    fn has_waiters(&self) -> bool {
        !self.fifo_queue.is_empty() || self.handle_waiters.values().any(|queue| !queue.is_empty())
    }

    /// Select one waiter according to the configured ordering policy.
    fn take_next_waiter(&mut self, policy: PoolFairnessPolicy) -> Option<WaiterSender<S, P, C>> {
        let waiter = match policy {
            PoolFairnessPolicy::RoundRobin => self.take_next_round_robin_waiter(),
            PoolFairnessPolicy::Fifo => self.take_next_fifo_waiter(),
        };
        if waiter.is_some() {
            self.servicing_waiters += 1;
        }
        waiter
    }

    /// Release one request after its lease is delivered or its receiver closes.
    fn finish_servicing_waiter(&mut self) {
        self.servicing_waiters = self.servicing_waiters.saturating_sub(1);
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
    /// Maximum number of acquisitions allowed to wait for a lease.
    max_waiters: usize,
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
    pub(crate) fn new(fairness_policy: PoolFairnessPolicy, max_waiters: usize) -> Self {
        Self {
            fairness_policy,
            max_waiters,
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
        match lock_or_recover(&self.state).enqueue_waiter(
            handle_id,
            sender,
            WaiterAdmission {
                fairness_policy: self.fairness_policy,
                max_waiters: self.max_waiters,
            },
        ) {
            WaiterEnqueueResult::Enqueued => {}
            WaiterEnqueueResult::UnregisteredHandle => {
                tracing::warn!(
                    handle_id,
                    fairness_policy = ?self.fairness_policy,
                    "pooled handle enqueue attempted for unregistered handle"
                );
                return Err(ClientError::disconnected());
            }
            WaiterEnqueueResult::Saturated => {
                return Err(ClientError::pool_saturated(self.max_waiters));
            }
        }

        if let Some(lease) = inner.try_acquire_immediately() {
            return self.resolve_immediate_lease(lease, receiver).await;
        }

        // No slot had a free permit, so this acquisition genuinely has to wait
        // for capacity. Recorded before parking: the wait exists from here on,
        // whether or not a worker has been spawned to service it yet.
        inner.churn.record_capacity_wait();
        self.kick(inner);
        receiver.await.map_err(|_| ClientError::disconnected())?
    }

    /// Complete every queued waiter with a disconnected error.
    pub(crate) fn notify_shutdown(&self) {
        while let Some(waiter) = self.next_waiter() {
            drop(waiter.send(Err(ClientError::disconnected())));
            self.finish_waiter();
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
            inner.churn.record_scheduler_spawn();
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

    /// Release capacity accounting after a selected waiter stops waiting.
    fn finish_waiter(&self) { lock_or_recover(&self.state).finish_servicing_waiter(); }

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

        self.finish_waiter();
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
}

mod service;
#[cfg(test)]
mod tests;
