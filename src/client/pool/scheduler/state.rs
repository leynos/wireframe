//! Owned fairness and waiter state for the persistent pool scheduler.
//!
//! The scheduler task is the only owner of this state. Keeping it as a plain
//! struct rather than a mutex-protected field removes the lock from the acquire
//! path: every command is handled by the task that owns the queues, so no other
//! thread can observe a half-applied transition.
//!
//! A waiter is a one-shot reply channel. Once the actor selects a waiter it
//! sends the outcome and forgets it; a waiter whose receiver has gone away is
//! pruned rather than handed a lease nobody can use.

use std::collections::{HashMap, VecDeque};

use tokio::sync::oneshot;

use super::super::{lease::PooledClientLease, policy::PoolFairnessPolicy};
use crate::{client::ClientError, serializer::Serializer};

/// One-shot completion channel for a queued logical-session acquisition.
pub(crate) type WaiterSender<S, P, C> =
    oneshot::Sender<Result<PooledClientLease<S, P, C>, ClientError>>;

/// Result of admitting one acquisition to the scheduler's waiter queues.
pub(crate) enum WaiterEnqueueResult<S, P, C>
where
    S: Serializer + Clone + Send + Sync + 'static,
    P: bincode::Encode + Clone + Send + Sync + 'static,
    C: Send + 'static,
{
    /// The request was queued and still needs capacity.
    Enqueued,
    /// The configured number of live waiters is already queued.
    ///
    /// The rejected sender is returned rather than dropped so the actor can
    /// resolve it with [`ClientError::PoolSaturated`]. Dropping it would close
    /// the caller's receiver and report a pool closure instead.
    Saturated {
        /// Channel that must receive the saturation error.
        rejected: WaiterSender<S, P, C>,
    },
}

/// Fairness and capacity settings applied to one waiter admission attempt.
#[derive(Clone, Copy)]
pub(crate) struct WaiterAdmission {
    /// Queue ordering policy for this pool.
    pub(crate) fairness_policy: PoolFairnessPolicy,
    /// Maximum live waiter count accepted by the pool.
    pub(crate) max_waiters: usize,
}

/// Fairness queues owned exclusively by the scheduler task.
pub(crate) struct SchedulerState<S, P, C>
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
    /// Start with empty queues and no known logical sessions.
    pub(crate) fn new() -> Self {
        Self {
            fifo_queue: VecDeque::new(),
            round_robin_order: VecDeque::new(),
            handle_waiters: HashMap::new(),
        }
    }

    /// Register a handle the first time it acquires capacity.
    ///
    /// Registration is lazy so constructing a `PoolHandle` performs no
    /// scheduler work and cannot race with the actor.
    fn register_handle(&mut self, handle_id: u64) {
        if let std::collections::hash_map::Entry::Vacant(entry) =
            self.handle_waiters.entry(handle_id)
        {
            entry.insert(VecDeque::new());
            self.round_robin_order.push_back(handle_id);
        }
    }

    /// Remove a handle and abandon its queued requests.
    ///
    /// Idempotent: a handle that was never registered and one deregistered
    /// twice are both harmless.
    pub(crate) fn deregister_handle(&mut self, handle_id: u64) {
        self.handle_waiters.remove(&handle_id);
        self.round_robin_order
            .retain(|queued_id| *queued_id != handle_id);
        self.fifo_queue
            .retain(|(queued_id, _)| *queued_id != handle_id);
    }

    /// Remove abandoned acquisitions from every policy queue.
    pub(crate) fn prune_cancelled_waiters(&mut self) {
        self.fifo_queue.retain(|(_, sender)| !sender.is_closed());
        for queue in self.handle_waiters.values_mut() {
            queue.retain(|sender| !sender.is_closed());
        }
    }

    /// Count queued requests, including ones already cancelled but not pruned.
    pub(crate) fn waiter_count(&self) -> usize {
        self.fifo_queue.len()
            + self
                .handle_waiters
                .values()
                .map(VecDeque::len)
                .sum::<usize>()
    }

    /// Report whether `handle_id` still has a request waiting for capacity.
    pub(crate) fn has_queued_waiter(&self, handle_id: u64) -> bool {
        self.handle_waiters
            .get(&handle_id)
            .is_some_and(|queue| !queue.is_empty())
            || self
                .fifo_queue
                .iter()
                .any(|(queued_id, _)| *queued_id == handle_id)
    }

    /// Report whether any queue holds a request whose caller still listens.
    pub(crate) fn has_live_waiters(&self) -> bool {
        self.fifo_queue
            .iter()
            .any(|(_, sender)| !sender.is_closed())
            || self
                .handle_waiters
                .values()
                .any(|queue| queue.iter().any(|sender| !sender.is_closed()))
    }

    /// Admit one acquisition, or hand the sender back when the pool is full.
    ///
    /// Pruning happens before the bound is checked, so a cancelled waiter
    /// releases admission capacity for a live one.
    pub(crate) fn enqueue_waiter(
        &mut self,
        handle_id: u64,
        sender: WaiterSender<S, P, C>,
        admission: WaiterAdmission,
    ) -> WaiterEnqueueResult<S, P, C> {
        let WaiterAdmission {
            fairness_policy,
            max_waiters,
        } = admission;
        self.prune_cancelled_waiters();
        self.register_handle(handle_id);

        if self.waiter_count() >= max_waiters {
            return WaiterEnqueueResult::Saturated { rejected: sender };
        }

        match fairness_policy {
            PoolFairnessPolicy::Fifo => {
                self.fifo_queue.push_back((handle_id, sender));
                WaiterEnqueueResult::Enqueued
            }
            PoolFairnessPolicy::RoundRobin => {
                // Registration above guarantees the entry exists; the fallback
                // still returns the sender rather than dropping it silently.
                match self.handle_waiters.get_mut(&handle_id) {
                    Some(queue) => {
                        queue.push_back(sender);
                        WaiterEnqueueResult::Enqueued
                    }
                    None => WaiterEnqueueResult::Saturated { rejected: sender },
                }
            }
        }
    }

    /// Select one waiter according to the configured ordering policy.
    ///
    /// Cancelled senders and requests from deregistered handles are skipped so
    /// no lease is ever handed to a caller that can no longer receive it.
    pub(crate) fn take_next_waiter(
        &mut self,
        policy: PoolFairnessPolicy,
    ) -> Option<WaiterSender<S, P, C>> {
        match policy {
            PoolFairnessPolicy::RoundRobin => self.take_next_round_robin_waiter(),
            PoolFairnessPolicy::Fifo => self.take_next_fifo_waiter(),
        }
    }

    /// Pop FIFO arrivals, skipping abandoned requests.
    fn take_next_fifo_waiter(&mut self) -> Option<WaiterSender<S, P, C>> {
        while let Some((handle_id, sender)) = self.fifo_queue.pop_front() {
            if sender.is_closed() || !self.handle_waiters.contains_key(&handle_id) {
                continue;
            }
            return Some(sender);
        }
        None
    }

    /// Visit each registered handle once and pop its oldest live request.
    ///
    /// The rotation advances one handle per selection, which is what gives
    /// each logical session a turn before any session gets a second one.
    fn take_next_round_robin_waiter(&mut self) -> Option<WaiterSender<S, P, C>> {
        let len = self.round_robin_order.len();
        for _ in 0..len {
            let handle_id = self.round_robin_order.pop_front()?;
            self.round_robin_order.push_back(handle_id);
            let Some(queue) = self.handle_waiters.get_mut(&handle_id) else {
                continue;
            };
            if let Some(sender) = pop_live_waiter(queue) {
                return Some(sender);
            }
        }
        None
    }
}

/// Pop the oldest request whose caller still listens, discarding abandoned ones.
fn pop_live_waiter<S, P, C>(
    queue: &mut VecDeque<WaiterSender<S, P, C>>,
) -> Option<WaiterSender<S, P, C>>
where
    S: Serializer + Clone + Send + Sync + 'static,
    P: bincode::Encode + Clone + Send + Sync + 'static,
    C: Send + 'static,
{
    while let Some(sender) = queue.pop_front() {
        if !sender.is_closed() {
            return Some(sender);
        }
    }
    None
}
