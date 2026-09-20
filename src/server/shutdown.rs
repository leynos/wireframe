//! Cloneable control for stopping and observing a running server.

use std::sync::Arc;

use tokio::sync::watch;
use tokio_util::sync::CancellationToken;

use super::ServerError;

/// Terminal state reported after the server supervisor has finished draining.
#[derive(Clone, Debug)]
pub(in crate::server) enum ServerTerminal {
    /// The supervisor closed and drained all tracked tasks.
    Clean,
    /// The supervisor task returned unexpectedly or panicked.
    Abnormal(String),
}

/// Request shutdown and await terminal server lifecycle state.
///
/// Cloning this handle creates another control endpoint, not another server
/// owner. Every clone observes the same single terminal outcome.
#[derive(Clone)]
pub struct ServerShutdown {
    /// Shared control and terminal observation state.
    inner: Arc<ServerShutdownInner>,
}

/// State shared by cloneable shutdown controls.
struct ServerShutdownInner {
    /// Level-triggered request observed by every accept loop.
    cancellation: CancellationToken,
    /// Terminal descriptor published by the join-handle observer.
    terminal: watch::Receiver<Option<ServerTerminal>>,
}

impl ServerShutdown {
    /// Assemble a handle from the supervisor's control and observation state.
    pub(in crate::server) fn new(
        cancellation: CancellationToken,
        terminal: watch::Receiver<Option<ServerTerminal>>,
    ) -> Self {
        Self {
            inner: Arc::new(ServerShutdownInner {
                cancellation,
                terminal,
            }),
        }
    }

    /// Request that every accept loop stop accepting new connections.
    ///
    /// This method is non-blocking and idempotent. Existing connection tasks
    /// continue to drain according to the server's normal graceful policy.
    pub fn stop(&self) { self.inner.cancellation.cancel(); }

    /// Wait until the listener is released and every accept loop has exited.
    ///
    /// A successful result means the supervisor closed and drained its task
    /// tracker. An abnormal supervisor termination is returned distinctly from
    /// a planned stop, with its captured diagnostic message.
    ///
    /// # Errors
    ///
    /// Returns [`ServerError::AbnormalTermination`] when the supervisor task
    /// ends unexpectedly or its observer ends without a terminal outcome.
    pub async fn drained(&self) -> Result<(), ServerError> {
        let mut terminal = self.inner.terminal.clone();

        loop {
            if let Some(outcome) = terminal.borrow_and_update().clone() {
                return terminal_result(outcome);
            }

            terminal
                .changed()
                .await
                .map_err(|_| ServerError::AbnormalTermination {
                    message: "server shutdown observer ended without a terminal outcome".to_owned(),
                })?;
        }
    }
}

/// Translate a shared terminal descriptor into the public result type.
fn terminal_result(terminal: ServerTerminal) -> Result<(), ServerError> {
    match terminal {
        ServerTerminal::Clean => Ok(()),
        ServerTerminal::Abnormal(message) => Err(ServerError::AbnormalTermination { message }),
    }
}
