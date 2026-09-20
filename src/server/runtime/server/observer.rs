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
    let terminal = match supervisor.await {
        Ok(Ok(())) => ServerTerminal::Clean,
        Ok(Err(error)) => ServerTerminal::Abnormal(error.to_string()),
        Err(error) => ServerTerminal::Abnormal(format_join_error(error)),
    };

    if let ServerTerminal::Abnormal(message) = &terminal {
        record_supervisor_abnormal_termination(message);
    }
    drop(terminal_tx.send_replace(Some(terminal)));
}

/// Preserve a panic payload when Tokio reports an abnormal task join.
fn format_join_error(error: JoinError) -> String {
    if error.is_panic() {
        format_panic(&error.into_panic())
    } else {
        error.to_string()
    }
}
