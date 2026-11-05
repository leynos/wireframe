//! Persistent scheduler task for one pooled client connection pool.
//!
//! Exactly one of these tasks exists per pool. It owns the fairness queues and
//! the pool's slot reference, and it is the only place waiter state changes.
//! Decisions live in synchronous handlers so they can be tested without a
//! spawned task; [`SchedulerTask::run`] is a thin loop that keeps the task's
//! borrows trivial.
//!
//! Capacity release is level-triggered rather than commanded. While waiters are
//! queued the task holds one asynchronous permit race, so a lease dropping its
//! permit wakes that race directly. There is no per-release message and no
//! per-release task.

use std::{future::Future, pin::Pin, sync::Arc};

use tokio::sync::mpsc;

use super::{
    command::SchedulerCommand,
    handle::SchedulerStatus,
    state::{SchedulerState, WaiterAdmission, WaiterEnqueueResult, WaiterSender},
};
use crate::{
    client::{
        ClientError,
        pool::{
            core::{PoolCore, SlotPermit},
            lease::PooledClientLease,
            policy::PoolFairnessPolicy,
        },
    },
    serializer::Serializer,
};

/// Boxed permit race borrowing the core's fixed slot storage.
type PermitRaceFuture<'a> =
    Pin<Box<dyn Future<Output = Result<SlotPermit, ClientError>> + Send + 'a>>;

/// One event selected by the scheduler loop.
enum ActorEvent<S, P, C>
where
    S: Serializer + Clone + Send + Sync + 'static,
    P: bincode::Encode + Clone + Send + Sync + 'static,
    C: Send + 'static,
{
    /// The pool published shutdown; drain and terminate.
    Shutdown,
    /// The in-flight permit race completed or failed.
    Permit(Option<Result<SlotPermit, ClientError>>),
    /// A command arrived, or the mailbox closed when the payload is `None`.
    Command(Option<SchedulerCommand<S, P, C>>),
}

/// The single in-flight race for slot capacity.
///
/// The future borrows a local clone of the core rather than a task field. That
/// keeps the task free of a self-referential borrow and makes cancellation a
/// matter of dropping this guard: the race future is dropped, releasing any
/// semaphore waiter it registered.
struct PendingGrant<'a> {
    /// Race future owned by the guard.
    future: PermitRaceFuture<'a>,
}

impl<'a> PendingGrant<'a> {
    /// Start a race against the supplied core.
    fn start<S, P, C>(core: &'a PoolCore<S, P, C>) -> Self
    where
        S: Serializer + Clone + Send + Sync + 'static,
        P: bincode::Encode + Clone + Send + Sync + 'static,
        C: Send + 'static,
    {
        Self {
            future: Box::pin(core.acquire_slot_permit()),
        }
    }
}

/// Await the pending race, or never complete when there is none.
async fn poll_pending_permit(
    pending: &mut Option<PendingGrant<'_>>,
) -> Option<Result<SlotPermit, ClientError>> {
    match pending {
        Some(grant) => Some(grant.future.as_mut().await),
        None => std::future::pending().await,
    }
}

/// Owner of one pool's fairness queues and command mailbox.
pub(crate) struct SchedulerTask<S, P, C>
where
    S: Serializer + Clone + Send + Sync + 'static,
    P: bincode::Encode + Clone + Send + Sync + 'static,
    C: Send + 'static,
{
    /// Pool reference held only for this task's lifetime.
    core: Arc<PoolCore<S, P, C>>,
    /// Lifecycle shared with handles.
    status: Arc<SchedulerStatus>,
    /// Fairness and waiter queues, owned outright.
    state: SchedulerState<S, P, C>,
    /// Command mailbox; closure means every handle is gone.
    receiver: mpsc::UnboundedReceiver<SchedulerCommand<S, P, C>>,
    /// Fairness ordering and the bound on blocked acquisitions.
    admission: WaiterAdmission,
}

impl<S, P, C> Drop for SchedulerTask<S, P, C>
where
    S: Serializer + Clone + Send + Sync + 'static,
    P: bincode::Encode + Clone + Send + Sync + 'static,
    C: Send + 'static,
{
    /// Publish an abnormal termination if the task unwinds.
    ///
    /// Drop glue runs this before the task's fields — including the receiver
    /// and the waiter queues — are released. Marking the status here therefore
    /// happens before queued callers observe their reply channels closing, so
    /// they report a scheduler failure rather than an orderly pool closure.
    fn drop(&mut self) {
        if std::thread::panicking() {
            self.status.mark_failed();
            crate::metrics::inc_pool_scheduler_failure();
            // The pool identity is unbounded across replacement pools, so it
            // belongs in the event rather than a metric label.
            tracing::error!(
                pool_id = self.status.pool_id(),
                "client pool scheduler task panicked; the pool will refuse new acquisitions"
            );
        }
    }
}

impl<S, P, C> SchedulerTask<S, P, C>
where
    S: Serializer + Clone + Send + Sync + 'static,
    P: bincode::Encode + Clone + Send + Sync + 'static,
    C: Send + 'static,
{
    /// Build a task that has not yet started serving.
    pub(crate) fn new(
        core: Arc<PoolCore<S, P, C>>,
        status: Arc<SchedulerStatus>,
        receiver: mpsc::UnboundedReceiver<SchedulerCommand<S, P, C>>,
        admission: WaiterAdmission,
    ) -> Self {
        Self {
            core,
            status,
            state: SchedulerState::new(),
            receiver,
            admission,
        }
    }

    /// Ordering policy applied when selecting the next waiter.
    const fn fairness_policy(&self) -> PoolFairnessPolicy { self.admission.fairness_policy }

    /// Serve commands until shutdown, mailbox closure, or an internal fault.
    pub(crate) async fn run(mut self) {
        let core = Arc::clone(&self.core);
        let mut pending: Option<PendingGrant<'_>> = None;

        loop {
            if pending.is_none() && self.state.has_live_waiters() {
                pending = Some(PendingGrant::start(core.as_ref()));
            }

            let event = tokio::select! {
                biased;
                () = core.shutdown_notified() => ActorEvent::Shutdown,
                permit = poll_pending_permit(&mut pending) => ActorEvent::Permit(permit),
                command = self.receiver.recv() => ActorEvent::Command(command),
            };

            match event {
                ActorEvent::Shutdown => {
                    // Returning drops the pending race and its borrowed core
                    // before the task's fields are released.
                    self.drain_for_shutdown();
                    return;
                }
                ActorEvent::Permit(outcome) => {
                    pending = None;
                    self.handle_permit_outcome(&core, outcome);
                }
                ActorEvent::Command(Some(command)) => self.handle_command(&core, command),
                ActorEvent::Command(None) => {
                    // Every handle is gone. Returning drops the race and the
                    // queues, then the task's fields release the pool
                    // reference.
                    self.status.mark_closed();
                    return;
                }
            }
        }
    }

    /// Apply one command to the owned state.
    fn handle_command(
        &mut self,
        core: &Arc<PoolCore<S, P, C>>,
        command: SchedulerCommand<S, P, C>,
    ) {
        match command {
            SchedulerCommand::Acquire { handle_id, reply } => {
                self.handle_acquire(core, handle_id, reply);
            }
            SchedulerCommand::DeregisterHandle { handle_id } => {
                self.state.deregister_handle(handle_id);
            }
            #[cfg(test)]
            SchedulerCommand::PanicForTest => {
                panic!("client pool scheduler panic injected by test")
            }
        }
    }

    /// Admit an acquisition and grant it immediately when capacity permits.
    fn handle_acquire(
        &mut self,
        core: &Arc<PoolCore<S, P, C>>,
        handle_id: u64,
        reply: WaiterSender<S, P, C>,
    ) {
        if !self.status.is_running() {
            // The caller may already be gone; a failed send has no recovery.
            drop(reply.send(Err(self.status.close_error())));
            return;
        }

        if let WaiterEnqueueResult::Saturated { rejected } =
            self.state.enqueue_waiter(handle_id, reply, self.admission)
        {
            // The rejected caller may already be gone; a failed send has no
            // recovery.
            drop(rejected.send(Err(ClientError::pool_saturated(self.admission.max_waiters))));
            return;
        }

        self.grant_sweep(core);
        // Only a request that is still queued after the sweep actually waited.
        if self.state.has_queued_waiter(handle_id) {
            self.status.churn().record_capacity_wait();
        }
    }

    /// Grant capacity to as many live waiters as the pool can satisfy now.
    fn grant_sweep(&mut self, core: &Arc<PoolCore<S, P, C>>) {
        while self.state.has_live_waiters() {
            let Some((slot_index, permit)) = core.try_acquire_permit() else {
                return;
            };
            let Some(waiter) = self.state.take_next_waiter(self.fairness_policy()) else {
                // No live waiter remains; dropping the permit returns it.
                return;
            };
            Self::deliver(core, waiter, slot_index, permit);
        }
    }

    /// Apply the result of the asynchronous permit race.
    fn handle_permit_outcome(
        &mut self,
        core: &Arc<PoolCore<S, P, C>>,
        outcome: Option<Result<SlotPermit, ClientError>>,
    ) {
        match outcome {
            Some(Ok(permit)) => {
                self.on_permit_won(core, permit);
                self.grant_sweep(core);
            }
            Some(Err(error)) => self.fail_on_race_error(&error),
            None => {}
        }
    }

    /// Hand one raced permit to the next waiter, or return it to the pool.
    fn on_permit_won(&mut self, core: &Arc<PoolCore<S, P, C>>, permit: SlotPermit) {
        let (slot_index, owned_permit) = permit;
        let Some(waiter) = self.state.take_next_waiter(self.fairness_policy()) else {
            return;
        };
        Self::deliver(core, waiter, slot_index, owned_permit);
    }

    /// Build a lease and send it, releasing it when the caller is gone.
    fn deliver(
        core: &Arc<PoolCore<S, P, C>>,
        waiter: WaiterSender<S, P, C>,
        slot_index: usize,
        permit: tokio::sync::OwnedSemaphorePermit,
    ) {
        match PooledClientLease::new(Arc::clone(core), slot_index, permit) {
            Some(lease) => {
                if let Err(Ok(returned)) = waiter.send(Ok(lease)) {
                    // The receiver went away between selection and delivery.
                    drop(returned);
                }
            }
            None => {
                // The caller may already be gone; a failed send has no
                // recovery.
                drop(waiter.send(Err(ClientError::scheduler_failed())));
            }
        }
    }

    /// Resolve every waiter and terminate after an unexpected race failure.
    ///
    /// A race error other than shutdown means the slot storage can no longer
    /// grant permits. The pool cannot recover, so callers are told the
    /// scheduler failed rather than being left parked forever.
    fn fail_on_race_error(&mut self, error: &ClientError) {
        if self.status.is_running() {
            self.status.mark_failed();
            crate::metrics::inc_pool_scheduler_failure();
            tracing::error!(
                pool_id = self.status.pool_id(),
                %error,
                "client pool permit race failed; refusing queued acquisitions"
            );
        }
        while let Some(waiter) = self.state.take_next_waiter(self.fairness_policy()) {
            // The caller may already be gone; a failed send has no recovery.
            drop(waiter.send(Err(self.status.close_error())));
        }
        self.receiver.close();
    }

    /// Resolve queued and buffered acquisitions before terminating.
    fn drain_for_shutdown(&mut self) {
        self.status.mark_closed();
        while let Some(waiter) = self.state.take_next_waiter(self.fairness_policy()) {
            // The caller may already be gone; a failed send has no recovery.
            drop(waiter.send(Err(ClientError::pool_closed())));
        }

        self.receiver.close();
        while let Ok(command) = self.receiver.try_recv() {
            if let SchedulerCommand::Acquire { reply, .. } = command {
                // The caller may already be gone; a failed send has no
                // recovery.
                drop(reply.send(Err(ClientError::pool_closed())));
            }
        }
    }
}

#[cfg(test)]
#[path = "actor_tests.rs"]
mod tests;
