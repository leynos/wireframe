//! Join-handle observation for the running server supervisor.

use tokio::{
    sync::watch,
    task::{JoinError, JoinHandle},
};

use super::super::supervisor::record_supervisor_abnormal_termination;
use crate::{
    panic::format_panic,
    server::{ServerError, ServerTerminal},
};

/// Retain the supervisor join handle and publish its one terminal outcome.
pub(in crate::server) async fn observe_supervisor_termination(
    supervisor: JoinHandle<Result<(), ServerError>>,
    terminal_tx: watch::Sender<Option<ServerTerminal>>,
) {
    let terminal = classify_supervisor_result(supervisor.await);
    record_terminal_observability(&terminal);
    drop(terminal_tx.send_replace(Some(terminal)));
}

/// Map the supervisor join result to the terminal outcome shared with callers.
///
/// For example, a normal `Ok(())` becomes `Clean`, while Tokio task
/// cancellation becomes `Cancelled` rather than a panic outcome.
fn classify_supervisor_result(
    result: Result<Result<(), ServerError>, JoinError>,
) -> ServerTerminal {
    match result {
        Ok(Ok(())) => ServerTerminal::Clean,
        Ok(Err(error)) => ServerTerminal::Abnormal(error.to_string()),
        Err(error) => match classify_join_error(error) {
            SupervisorJoinOutcome::Panicked(message) => ServerTerminal::Abnormal(message),
            SupervisorJoinOutcome::Cancelled => ServerTerminal::Cancelled,
        },
    }
}

/// Emit telemetry that matches the supervisor's terminal classification.
///
/// For example, a panic increments the abnormal-termination counter, while a
/// cancelled task emits its separate event without incrementing that counter.
fn record_terminal_observability(terminal: &ServerTerminal) {
    match terminal {
        ServerTerminal::Abnormal(message) => record_supervisor_abnormal_termination(message),
        ServerTerminal::Cancelled => tracing::error!(
            event = "server_supervisor_cancelled",
            "server supervisor task was cancelled before a clean shutdown"
        ),
        ServerTerminal::Clean => {}
    }
}

/// Distinguish a cancelled supervisor from a panic before formatting its payload.
enum SupervisorJoinOutcome {
    /// The supervisor panicked with this captured diagnostic.
    Panicked(String),
    /// Tokio cancelled the supervisor task before it completed.
    Cancelled,
}

/// Classify a failed task join and format a panic payload when present.
fn classify_join_error(error: JoinError) -> SupervisorJoinOutcome {
    if error.is_cancelled() {
        SupervisorJoinOutcome::Cancelled
    } else {
        debug_assert!(error.is_panic());
        SupervisorJoinOutcome::Panicked(format_panic(&error.into_panic()))
    }
}
