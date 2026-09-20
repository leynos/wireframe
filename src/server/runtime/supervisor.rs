//! Server-supervisor ownership, cancellation, and terminal observation.

use std::sync::{
    Arc,
    atomic::{AtomicU8, Ordering},
};

use futures::Future;
use tokio::select;
use tokio_util::{
    sync::{CancellationToken, DropGuardRef},
    task::TaskTracker,
};

use crate::metrics::{
    ServerCancellationReason,
    inc_server_supervisor_abnormal_termination,
    inc_server_supervisor_cancellation,
};

/// Supervisor has not yet observed completion or cancellation.
const SUPERVISOR_RUNNING: u8 = 0;
/// Shutdown was requested through the supplied shutdown future.
const SUPERVISOR_GRACEFUL: u8 = 1;
/// The foreground supervisor future was dropped before shutdown completed.
const SUPERVISOR_DROPPED: u8 = 2;
/// All tracked worker tasks completed before a shutdown request.
const SUPERVISOR_FINISHED: u8 = 3;

/// Shares one terminal cancellation reason with the supervisor's accept loops.
#[derive(Clone, Debug)]
pub(in crate::server) struct SupervisorLifecycle {
    /// Atomic cancellation state shared only for bounded observability labels.
    state: Arc<AtomicU8>,
}

impl SupervisorLifecycle {
    /// Start lifecycle observation before worker tasks are created.
    pub(super) fn new() -> Self {
        Self {
            state: Arc::new(AtomicU8::new(SUPERVISOR_RUNNING)),
        }
    }

    /// Record intentional cancellation before notifying the accept loops.
    pub(super) fn record_graceful_cancellation(&self) {
        self.state.store(SUPERVISOR_GRACEFUL, Ordering::Release);
        record_supervisor_cancellation(ServerCancellationReason::Graceful);
    }

    /// Record cancellation caused by the foreground supervisor being dropped.
    pub(super) fn record_dropped_cancellation(&self) {
        if self
            .state
            .compare_exchange(
                SUPERVISOR_RUNNING,
                SUPERVISOR_DROPPED,
                Ordering::AcqRel,
                Ordering::Acquire,
            )
            .is_ok()
        {
            record_supervisor_cancellation(ServerCancellationReason::Dropped);
        }
    }

    /// Record that all tracked workers ended without a cancellation request.
    pub(super) fn record_completion_without_cancellation(&self) {
        self.state.store(SUPERVISOR_FINISHED, Ordering::Release);
    }

    /// Translate the atomic terminal state into the metrics-facing reason.
    pub(in crate::server) fn cancellation_reason(&self) -> ServerCancellationReason {
        match self.state.load(Ordering::Acquire) {
            SUPERVISOR_GRACEFUL => ServerCancellationReason::Graceful,
            SUPERVISOR_DROPPED | SUPERVISOR_RUNNING | SUPERVISOR_FINISHED => {
                ServerCancellationReason::Dropped
            }
            _ => ServerCancellationReason::Dropped,
        }
    }
}

/// Emit observability for one bounded supervisor cancellation reason.
fn record_supervisor_cancellation(reason: ServerCancellationReason) {
    inc_server_supervisor_cancellation(reason);
    tracing::info!(
        event = "server_supervisor_cancellation",
        reason = reason.as_str(),
        "server_supervisor_cancellation",
    );
}

/// Emit observability for an unexpected supervisor terminal outcome.
pub(super) fn record_supervisor_abnormal_termination(message: &str) {
    inc_server_supervisor_abnormal_termination();
    tracing::error!(
        event = "server_supervisor_abnormal_termination",
        panic_message = %message,
        "server supervisor terminated abnormally",
    );
}

/// Cancels accept loops when its supervisor frame is abandoned.
pub(super) struct SupervisorCancellationDropGuard<'a> {
    /// Tokio cancellation guard that fires when the supervisor future drops.
    _guard: DropGuardRef<'a>,
    /// Shared lifecycle state updated before the Tokio guard cancels workers.
    lifecycle: SupervisorLifecycle,
}

impl<'a> SupervisorCancellationDropGuard<'a> {
    /// Pair Tokio's cancellation guard with lifecycle accounting.
    pub(super) fn new(guard: DropGuardRef<'a>, lifecycle: SupervisorLifecycle) -> Self {
        Self {
            _guard: guard,
            lifecycle,
        }
    }
}

impl Drop for SupervisorCancellationDropGuard<'_> {
    fn drop(&mut self) { self.lifecycle.record_dropped_cancellation(); }
}

/// Wait for deliberate shutdown or for all tracked workers to complete.
#[expect(
    clippy::integer_division_remainder_used,
    reason = "tokio::select! expands to modulus internally"
)]
pub(super) async fn await_supervisor_termination<S>(
    shutdown: S,
    shutdown_token: &CancellationToken,
    tracker: &TaskTracker,
    lifecycle: &SupervisorLifecycle,
) where
    S: Future<Output = ()>,
{
    select! {
        () = shutdown => {
            lifecycle.record_graceful_cancellation();
            shutdown_token.cancel();
        }
        () = tracker.wait() => lifecycle.record_completion_without_cancellation(),
    }
}
