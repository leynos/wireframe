//! Command protocol accepted by the persistent client-pool scheduler.
//!
//! The mailbox is the only way to reach scheduler state. Closing the mailbox
//! is therefore also the shutdown signal for the implicit case: when the final
//! [`SchedulerHandle`](super::SchedulerHandle) is dropped, the task observes a
//! closed receiver and releases its pool reference.
//!
//! Capacity release deliberately has no command. A released permit is observed
//! by the task's in-flight permit race, which keeps the protocol small and
//! avoids a per-release message that could arrive after shutdown.

use super::state::WaiterSender;
use crate::serializer::Serializer;

/// One message delivered to the pool scheduler task.
pub(crate) enum SchedulerCommand<S, P, C>
where
    S: Serializer + Clone + Send + Sync + 'static,
    P: bincode::Encode + Clone + Send + Sync + 'static,
    C: Send + 'static,
{
    /// Queue one acquisition for a logical handle.
    Acquire {
        /// Logical session asking for a lease.
        handle_id: u64,
        /// Completion channel receiving the lease or a typed refusal.
        reply: WaiterSender<S, P, C>,
    },
    /// Remove a handle and abandon any requests it left queued.
    DeregisterHandle {
        /// Logical session that will not acquire again.
        handle_id: u64,
    },
    /// Panic the scheduler task to prove failure is visible to callers.
    ///
    /// Only the scheduler's own tests can send this; it exists so the
    /// termination path can be exercised without a real internal fault.
    #[cfg(test)]
    PanicForTest,
}
