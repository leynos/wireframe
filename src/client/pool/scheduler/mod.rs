//! Persistent fair-lease scheduling for pooled client handles.
//!
//! Each pool owns exactly one [`SchedulerTask`]. The task owns the fairness
//! queues and a reference to the fixed physical slots, and every acquisition is
//! a command to its mailbox. That is what removes the per-release task spawn
//! and the shared scheduler mutex: all waiter state changes happen while the
//! task runs, so no other thread can observe a partially applied transition.
//!
//! The module is laid out by responsibility:
//!
//! - [`state`] holds the owned fairness queues.
//! - [`command`] defines the mailbox protocol.
//! - [`handle`] is the cloneable sender plus the shared lifecycle record.
//! - [`actor`] is the task loop and its synchronous decision handlers.

mod actor;
mod command;
mod handle;
mod state;

use std::sync::Arc;

use actor::SchedulerTask;
pub(crate) use handle::SchedulerHandle;
use handle::SchedulerStatus;
use state::WaiterAdmission;
use tokio::{sync::mpsc, task::JoinHandle};

use super::{churn::PoolTaskChurnCounts, core::PoolCore, policy::PoolFairnessPolicy};
use crate::serializer::Serializer;

/// Start the single scheduler task that serves `core`.
///
/// The returned join handle finishes when the task observes shutdown, the
/// mailbox closes, or an internal fault stops it. Dropping the handle instead
/// of awaiting it is safe if the pool is being torn down without an orderly
/// [`WireframeClientPool::close`](super::client_pool::WireframeClientPool::close).
pub(crate) fn start<S, P, C>(
    core: &Arc<PoolCore<S, P, C>>,
    fairness_policy: PoolFairnessPolicy,
    max_waiters: usize,
    churn: Arc<PoolTaskChurnCounts>,
) -> (SchedulerHandle<S, P, C>, JoinHandle<()>)
where
    S: Serializer + Clone + Send + Sync + 'static,
    P: bincode::Encode + Clone + Send + Sync + 'static,
    C: Send + 'static,
{
    let (sender, receiver) = mpsc::unbounded_channel();
    let status = Arc::new(SchedulerStatus::new(churn));
    // One spawn per pool, recorded where it happens so the churn observer can
    // distinguish construction from acquisition work.
    status.churn().record_scheduler_spawn();
    let admission = WaiterAdmission {
        fairness_policy,
        max_waiters,
    };
    let task = SchedulerTask::new(Arc::clone(core), Arc::clone(&status), receiver, admission);
    let join = tokio::spawn(task.run());
    (SchedulerHandle::new(sender, status), join)
}

#[cfg(test)]
mod tests;
