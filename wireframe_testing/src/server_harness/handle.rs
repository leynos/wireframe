//! The running-server handle.

use std::{fmt, net::SocketAddr};

use tokio::time::timeout;
use wireframe::server::ServerShutdown;

use super::{
    options::ServerHarnessOptions,
    outcome::{self, ShutdownOutcome},
};
use crate::TestResult;

/// A bound, ready, running Wireframe server owned by a test.
///
/// The handle owns exactly one [`ServerShutdown`] and never clones or exposes
/// it, so dropping the handle drops the last control and requests shutdown.
/// There is deliberately no `impl Drop`: dropping the control is the whole of
/// the cleanup.
///
/// Dropping this handle requests shutdown, so binding it to `_` stops a server
/// you probably meant to keep.
#[must_use]
pub struct RunningWireframeServer {
    /// Address captured from the listener before the server was bound.
    local_addr: SocketAddr,
    /// Server control owning the running supervisor.
    control: ServerShutdown,
    /// Timeout policy applied to readiness and shutdown.
    options: ServerHarnessOptions,
    /// Most recent shutdown outcome, or `None` before any call has concluded.
    outcome: Option<ShutdownOutcome>,
}

impl RunningWireframeServer {
    /// Assemble a handle from a ready server, its address, and its policy.
    pub(crate) const fn new(
        local_addr: SocketAddr,
        control: ServerShutdown,
        options: ServerHarnessOptions,
    ) -> Self {
        Self {
            local_addr,
            control,
            options,
            outcome: None,
        }
    }

    /// The address the server was bound to.
    #[must_use]
    pub const fn local_addr(&self) -> SocketAddr { self.local_addr }

    /// The most recent shutdown outcome.
    ///
    /// [`None`] means no shutdown call has concluded yet.
    #[must_use]
    pub const fn shutdown_outcome(&self) -> Option<ShutdownOutcome> { self.outcome }

    /// Request shutdown and wait, under the configured bound, for the drain.
    ///
    /// The call is idempotent in effect: every call issues the server's
    /// non-blocking stop, and a call after the drain has concluded returns an
    /// equivalent result. After a timeout, a later call re-checks with a zero
    /// bound instead of blocking again.
    ///
    /// # Errors
    ///
    /// Returns [`TestError::Server`] carrying
    /// [`ServerError::AbnormalTermination`] if the server terminated
    /// abnormally, or [`TestError::Timeout`] if the drain did not finish
    /// within the bound.
    ///
    /// [`TestError::Server`]: crate::TestError::Server
    /// [`TestError::Timeout`]: crate::TestError::Timeout
    /// [`ServerError::AbnormalTermination`]: wireframe::server::ServerError::AbnormalTermination
    pub async fn shutdown(&mut self) -> TestResult<()> {
        self.control.stop();
        let drain_bound = outcome::drain_bound(self.outcome, self.options.shutdown_timeout());
        let drained = match drain_bound {
            Some(limit) => timeout(limit, self.control.drained()).await,
            None => Ok(self.control.drained().await),
        };
        let (observed, result) = outcome::classify_drain(drained);
        self.outcome = Some(outcome::merge_outcome(self.outcome, observed));
        result
    }
}

impl fmt::Debug for RunningWireframeServer {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("RunningWireframeServer")
            .field("local_addr", &self.local_addr)
            .field("shutdown_outcome", &self.outcome)
            .finish_non_exhaustive()
    }
}
