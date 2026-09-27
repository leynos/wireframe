//! Cloneable control for stopping and observing a running server.

use std::sync::Arc;

use tokio::sync::watch;
use tokio_util::sync::{CancellationToken, DropGuard};

use super::ServerError;

/// Terminal state published after the server supervisor exits.
#[derive(Clone, Debug)]
pub(in crate::server) enum ServerTerminal {
    /// The supervisor closed and drained all tracked tasks.
    Clean,
    /// The supervisor task returned unexpectedly or panicked.
    Abnormal(String),
    /// Tokio cancelled the supervisor task before it completed.
    Cancelled,
}

/// Request shutdown and await terminal server lifecycle state.
///
/// Cloning this handle creates another control endpoint, not another server
/// owner. Every clone observes the same single terminal outcome.
/// Dropping the final clone requests shutdown; keep a clone alive while
/// awaiting [`Self::drained`] when the terminal outcome matters.
#[derive(Clone)]
pub struct ServerShutdown {
    /// Shared control and terminal observation state.
    inner: Arc<ServerShutdownInner>,
}

/// State shared by cloneable shutdown controls.
struct ServerShutdownInner {
    /// Level-triggered request observed by the supervisor before it cancels workers.
    stop_requested: CancellationToken,
    /// Cancels the supervisor when the final shared control is dropped.
    _stop_guard: DropGuard,
    /// Terminal descriptor published by the join-handle observer.
    terminal: watch::Receiver<Option<ServerTerminal>>,
}

impl ServerShutdown {
    /// Assemble a handle from the supervisor's control and observation state.
    pub(in crate::server) fn new(
        stop_requested: CancellationToken,
        terminal: watch::Receiver<Option<ServerTerminal>>,
    ) -> Self {
        let stop_guard = stop_requested.clone().drop_guard();
        Self {
            inner: Arc::new(ServerShutdownInner {
                stop_requested,
                _stop_guard: stop_guard,
                terminal,
            }),
        }
    }

    /// Request that every accept loop stop accepting new connections.
    ///
    /// This method is non-blocking and idempotent. Existing connection tasks
    /// continue to drain according to the server's normal graceful policy.
    ///
    /// # Examples
    ///
    /// `stop` requests a clean drain; `drained` waits until that outcome is
    /// published:
    ///
    /// ```
    /// use wireframe::{app::WireframeApp, server::WireframeServer};
    ///
    /// #[tokio::main]
    /// async fn main() -> Result<(), Box<dyn std::error::Error>> {
    ///     let server = WireframeServer::new(|| -> WireframeApp { WireframeApp::default() })
    ///         .bind("127.0.0.1:0".parse()?)?;
    ///     let shutdown = server.spawn().await?;
    ///     shutdown.stop();
    ///     shutdown.drained().await?;
    ///     Ok(())
    /// }
    /// ```
    pub fn stop(&self) { self.inner.stop_requested.cancel(); }

    /// Wait for the server supervisor's terminal outcome.
    ///
    /// A successful result guarantees that every accept loop has exited, the
    /// listener is released, and the supervisor has drained its task tracker.
    /// An abnormal termination is returned with its captured diagnostic, but
    /// does not by itself prove that the drain completed.
    ///
    /// # Examples
    ///
    /// After requesting shutdown, awaiting this method returns `Ok(())` once
    /// the clean drain has completed:
    ///
    /// ```
    /// use wireframe::{app::WireframeApp, server::WireframeServer};
    ///
    /// #[tokio::main]
    /// async fn main() -> Result<(), Box<dyn std::error::Error>> {
    ///     let server = WireframeServer::new(|| -> WireframeApp { WireframeApp::default() })
    ///         .bind("127.0.0.1:0".parse()?)?;
    ///     let shutdown = server.spawn().await?;
    ///     shutdown.stop();
    ///     shutdown.drained().await?;
    ///     Ok(())
    /// }
    /// ```
    ///
    /// # Errors
    ///
    /// Returns [`ServerError::AbnormalTermination`] when the supervisor task
    /// fails, is cancelled, or its observer ends without a terminal outcome.
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
        ServerTerminal::Cancelled => Err(ServerError::AbnormalTermination {
            message: "server supervisor task was cancelled before a clean shutdown".to_owned(),
        }),
    }
}
