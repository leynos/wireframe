//! Fair lease scheduling for pooled client handles.

use std::{
    collections::{HashMap, VecDeque},
    sync::{
        Arc,
        Mutex,
        MutexGuard,
        atomic::{AtomicBool, AtomicU64, Ordering},
    },
};

use tokio::sync::oneshot;

use super::{client_pool::ClientPoolInner, lease::PooledClientLease, policy::PoolFairnessPolicy};
use crate::{client::ClientError, serializer::Serializer};

fn recover_mutex<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    match mutex.lock() {
        Ok(guard) => guard,
        Err(poisoned) => poisoned.into_inner(),
    }
}

type WaiterSender<S, P, C> = oneshot::Sender<Result<PooledClientLease<S, P, C>, ClientError>>;

struct SchedulerState<S, P, C>
where
    S: Serializer + Clone + Send + Sync + 'static,
    P: bincode::Encode + Clone + Send + Sync + 'static,
    C: Send + 'static,
{
    waiters: HashMap<u64, WaiterSender<S, P, C>>,
    fifo_waiters: VecDeque<u64>,
    round_robin_handles: VecDeque<u64>,
}

impl<S, P, C> SchedulerState<S, P, C>
where
    S: Serializer + Clone + Send + Sync + 'static,
    P: bincode::Encode + Clone + Send + Sync + 'static,
    C: Send + 'static,
{
    fn new() -> Self {
        Self {
            waiters: HashMap::new(),
            fifo_waiters: VecDeque::new(),
            round_robin_handles: VecDeque::new(),
        }
    }

    fn register_handle(&mut self, handle_id: u64) { self.round_robin_handles.push_back(handle_id); }

    fn deregister_handle(&mut self, handle_id: u64) {
        self.waiters.remove(&handle_id);
        self.fifo_waiters
            .retain(|queued_id| *queued_id != handle_id);
        self.round_robin_handles
            .retain(|queued_id| *queued_id != handle_id);
    }

    fn enqueue_waiter(&mut self, handle_id: u64, sender: WaiterSender<S, P, C>) {
        if self.waiters.insert(handle_id, sender).is_none() {
            self.fifo_waiters.push_back(handle_id);
        }
    }

    fn has_waiters(&self) -> bool { !self.waiters.is_empty() }

    fn take_next_waiter(&mut self, policy: PoolFairnessPolicy) -> Option<WaiterSender<S, P, C>> {
        match policy {
            PoolFairnessPolicy::RoundRobin => self.take_next_round_robin_waiter(),
            PoolFairnessPolicy::Fifo => self.take_next_fifo_waiter(),
        }
    }

    fn take_next_fifo_waiter(&mut self) -> Option<WaiterSender<S, P, C>> {
        while let Some(handle_id) = self.fifo_waiters.pop_front() {
            if let Some(sender) = self.waiters.remove(&handle_id) {
                return Some(sender);
            }
        }
        None
    }

    fn take_next_round_robin_waiter(&mut self) -> Option<WaiterSender<S, P, C>> {
        let len = self.round_robin_handles.len();
        for _ in 0..len {
            let handle_id = self.round_robin_handles.pop_front()?;
            self.round_robin_handles.push_back(handle_id);
            if let Some(sender) = self.waiters.remove(&handle_id) {
                self.fifo_waiters
                    .retain(|queued_id| *queued_id != handle_id);
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
    fairness_policy: PoolFairnessPolicy,
    next_handle_id: AtomicU64,
    is_servicing: AtomicBool,
    state: Mutex<SchedulerState<S, P, C>>,
}

impl<S, P, C> PoolScheduler<S, P, C>
where
    S: Serializer + Clone + Send + Sync + 'static,
    P: bincode::Encode + Clone + Send + Sync + 'static,
    C: Send + 'static,
{
    pub(crate) fn new(fairness_policy: PoolFairnessPolicy) -> Self {
        Self {
            fairness_policy,
            next_handle_id: AtomicU64::new(1),
            is_servicing: AtomicBool::new(false),
            state: Mutex::new(SchedulerState::new()),
        }
    }

    pub(crate) fn register_handle(&self) -> u64 {
        let handle_id = self.next_handle_id.fetch_add(1, Ordering::Relaxed);
        recover_mutex(&self.state).register_handle(handle_id);
        handle_id
    }

    pub(crate) fn deregister_handle(&self, handle_id: u64) {
        recover_mutex(&self.state).deregister_handle(handle_id);
    }

    pub(crate) async fn acquire_for_handle(
        self: &Arc<Self>,
        inner: Arc<ClientPoolInner<S, P, C>>,
        handle_id: u64,
    ) -> Result<PooledClientLease<S, P, C>, ClientError> {
        if inner.is_shutdown() {
            return Err(ClientError::disconnected());
        }

        let (sender, receiver) = oneshot::channel();
        recover_mutex(&self.state).enqueue_waiter(handle_id, sender);

        if let Some(lease) = inner.try_acquire_immediately() {
            let Some(waiter) = self.take_next_waiter_or_stop() else {
                drop(receiver);
                return Ok(lease);
            };

            if waiter.send(Ok(lease)).is_err() {
                drop(receiver);
                return Err(ClientError::disconnected());
            }
        } else {
            self.kick(inner);
        }

        receiver.await.map_err(|_| ClientError::disconnected())?
    }

    pub(crate) fn notify_shutdown(&self) {
        let mut state = recover_mutex(&self.state);
        while let Some(waiter) = state.take_next_waiter(self.fairness_policy) {
            let _ = waiter.send(Err(ClientError::disconnected()));
        }
    }

    pub(crate) fn notify_capacity_available(
        self: &Arc<Self>,
        inner: Arc<ClientPoolInner<S, P, C>>,
    ) {
        self.kick(inner);
    }

    fn kick(self: &Arc<Self>, inner: Arc<ClientPoolInner<S, P, C>>) {
        if self
            .is_servicing
            .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
            .is_ok()
        {
            let scheduler = Arc::clone(self);
            tokio::spawn(async move {
                scheduler.service_waiters(inner).await;
            });
        }
    }

    fn restart_if_waiters(&self) -> bool {
        if !recover_mutex(&self.state).has_waiters() {
            return false;
        }

        self.is_servicing
            .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
            .is_ok()
    }

    fn take_next_waiter_or_stop(&self) -> Option<WaiterSender<S, P, C>> {
        loop {
            if let Some(sender) = recover_mutex(&self.state).take_next_waiter(self.fairness_policy)
            {
                return Some(sender);
            }

            self.is_servicing.store(false, Ordering::Release);
            if !self.restart_if_waiters() {
                return None;
            }
        }
    }

    async fn service_waiters(self: Arc<Self>, inner: Arc<ClientPoolInner<S, P, C>>) {
        while let Some(sender) = self.take_next_waiter_or_stop() {
            self.service_one_waiter(sender, Arc::clone(&inner)).await;
        }
    }

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
