//! Cloneable command handle for the persistent client-pool scheduler.
//!
//! A [`SchedulerHandle`] owns only a mailbox sender and the small shared status
//! record every clone needs: the logical-handle identity counter and the
//! lifecycle that decides which typed error a refused acquisition reports. It
//! holds no pool reference and no receiver, so a handle can never keep the
//! scheduler task or the physical slots alive by a cycle.

use std::sync::{
    Arc,
    atomic::{AtomicU8, AtomicU64, Ordering},
};

use tokio::sync::{mpsc, oneshot};

use super::command::SchedulerCommand;
use crate::{
    client::{
        ClientError,
        pool::{churn::PoolTaskChurnCounts, lease::PooledClientLease},
    },
    serializer::Serializer,
};

/// Process-local identity source for pools that report scheduler failures.
static NEXT_POOL_ID: AtomicU64 = AtomicU64::new(1);

/// Lifecycle of the scheduler task behind one pool.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(u8)]
pub(crate) enum SchedulerLifecycle {
    /// The task is accepting commands and may grant leases.
    Running = 0,
    /// The pool closed in an orderly way.
    Closed = 1,
    /// The task terminated abnormally.
    Failed = 2,
}

impl SchedulerLifecycle {
    /// Decode a stored lifecycle, treating unknown values as failure.
    fn from_raw(raw: u8) -> Self {
        match raw {
            0 => Self::Running,
            1 => Self::Closed,
            _ => Self::Failed,
        }
    }
}

/// State shared by every clone of a [`SchedulerHandle`].
///
/// This record must stay free of senders, receivers, and pool references:
/// anything rooted here is reachable from every handle and would become a
/// shutdown cycle.
pub(crate) struct SchedulerStatus {
    /// Monotonic source of logical-session identities.
    next_handle_id: AtomicU64,
    /// Current lifecycle, stored as a [`SchedulerLifecycle`] discriminant.
    lifecycle: AtomicU8,
    /// Task-churn counters scoped to this pool.
    churn: Arc<PoolTaskChurnCounts>,
    /// Process-local identifier used for structured failure logging.
    pool_id: u64,
}

impl SchedulerStatus {
    /// Create running status for one newly spawned pool scheduler.
    pub(crate) fn new(churn: Arc<PoolTaskChurnCounts>) -> Self {
        Self {
            next_handle_id: AtomicU64::new(1),
            lifecycle: AtomicU8::new(SchedulerLifecycle::Running as u8),
            churn,
            pool_id: NEXT_POOL_ID.fetch_add(1, Ordering::Relaxed),
        }
    }

    /// Allocate the next logical-handle identity.
    pub(crate) fn allocate_handle_id(&self) -> u64 {
        self.next_handle_id.fetch_add(1, Ordering::Relaxed)
    }

    /// Read the current lifecycle.
    pub(crate) fn lifecycle(&self) -> SchedulerLifecycle {
        SchedulerLifecycle::from_raw(self.lifecycle.load(Ordering::Acquire))
    }

    /// Report whether the scheduler still accepts acquisitions.
    pub(crate) fn is_running(&self) -> bool { self.lifecycle() == SchedulerLifecycle::Running }

    /// Publish an orderly closure without overwriting an earlier failure.
    pub(crate) fn mark_closed(&self) {
        let _ = self.lifecycle.compare_exchange(
            SchedulerLifecycle::Running as u8,
            SchedulerLifecycle::Closed as u8,
            Ordering::AcqRel,
            Ordering::Acquire,
        );
    }

    /// Publish an abnormal termination. A failure is final.
    pub(crate) fn mark_failed(&self) {
        self.lifecycle
            .store(SchedulerLifecycle::Failed as u8, Ordering::Release);
    }

    /// Typed error describing why a refused acquisition cannot proceed.
    pub(crate) fn close_error(&self) -> ClientError {
        match self.lifecycle() {
            SchedulerLifecycle::Failed => ClientError::scheduler_failed(),
            SchedulerLifecycle::Running | SchedulerLifecycle::Closed => ClientError::pool_closed(),
        }
    }

    /// Counters describing this pool's scheduler activity.
    pub(crate) fn churn(&self) -> &PoolTaskChurnCounts { &self.churn }

    /// Process-local pool identifier, used for structured failure logging.
    pub(crate) fn pool_id(&self) -> u64 { self.pool_id }
}

/// Cloneable handle used by pool owners and logical sessions to reach the task.
pub(crate) struct SchedulerHandle<S, P, C>
where
    S: Serializer + Clone + Send + Sync + 'static,
    P: bincode::Encode + Clone + Send + Sync + 'static,
    C: Send + 'static,
{
    /// Mailbox sender; dropping the last clone ends the task.
    sender: mpsc::UnboundedSender<SchedulerCommand<S, P, C>>,
    /// Lifecycle and identity state shared with every clone.
    status: Arc<SchedulerStatus>,
}

impl<S, P, C> Clone for SchedulerHandle<S, P, C>
where
    S: Serializer + Clone + Send + Sync + 'static,
    P: bincode::Encode + Clone + Send + Sync + 'static,
    C: Send + 'static,
{
    fn clone(&self) -> Self {
        Self {
            sender: self.sender.clone(),
            status: Arc::clone(&self.status),
        }
    }
}

impl<S, P, C> SchedulerHandle<S, P, C>
where
    S: Serializer + Clone + Send + Sync + 'static,
    P: bincode::Encode + Clone + Send + Sync + 'static,
    C: Send + 'static,
{
    /// Build a handle around a mailbox and its shared status.
    pub(crate) const fn new(
        sender: mpsc::UnboundedSender<SchedulerCommand<S, P, C>>,
        status: Arc<SchedulerStatus>,
    ) -> Self {
        Self { sender, status }
    }

    /// Allocate a logical-handle identity without touching the task.
    pub(crate) fn allocate_handle_id(&self) -> u64 { self.status.allocate_handle_id() }

    /// Queue an acquisition and await its lease or typed refusal.
    ///
    /// A send or reply failure means the task is gone; the status record, not
    /// the channel error, decides whether that was a closure or a failure.
    pub(crate) async fn acquire(
        &self,
        handle_id: u64,
    ) -> Result<PooledClientLease<S, P, C>, ClientError> {
        if !self.status.is_running() {
            return Err(self.status.close_error());
        }

        let (reply, receiver) = oneshot::channel();
        if self
            .sender
            .send(SchedulerCommand::Acquire { handle_id, reply })
            .is_err()
        {
            return Err(self.status.close_error());
        }
        receiver.await.map_err(|_| self.status.close_error())?
    }

    /// Ask the task to forget a logical session.
    ///
    /// Non-blocking and harmless after shutdown: a closed mailbox already
    /// implies the task has no waiter left to clean up.
    pub(crate) fn deregister(&self, handle_id: u64) {
        let _ = self
            .sender
            .send(SchedulerCommand::DeregisterHandle { handle_id });
    }

    /// Publish an orderly closure before the mailbox is dropped.
    pub(crate) fn mark_closed(&self) { self.status.mark_closed(); }

    /// Clone the pool's task-churn counters for an observer.
    pub(crate) fn churn(&self) -> Arc<PoolTaskChurnCounts> { Arc::clone(&self.status.churn) }

    /// Inject a scheduler panic so tests can observe failure propagation.
    #[cfg(test)]
    pub(crate) fn panic_for_test(&self) {
        let _ = self.sender.send(SchedulerCommand::PanicForTest);
    }
}
