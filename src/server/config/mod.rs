//! Configuration utilities for [`WireframeServer`].
//!
//! Provides a fluent builder for configuring `WireframeServer` instances.
//! The builder exposes worker count tuning, preamble callbacks,
//! ready-signal configuration, and TCP binding. The server may be constructed
//! unbound and later bound via [`bind`](WireframeServer::bind).

use core::marker::PhantomData;
use tokio::sync::oneshot;

use super::{ServerState, Unbound, WireframeServer};
use crate::{app::WireframeApp, preamble::Preamble};

mod binding;
mod preamble;

impl<F> WireframeServer<F, (), Unbound>
where
    F: Fn() -> WireframeApp + Send + Sync + Clone + 'static,
{
    /// Create a new `WireframeServer` from the given application factory.
    ///
    /// The worker count defaults to the number of available CPU cores (or 1 if
    /// this cannot be determined). The server is initially unbound; call
    /// `bind` (available on unbound servers) before running the server.
    ///
    /// # Examples
    ///
    /// ```
    /// use wireframe::{app::WireframeApp, server::WireframeServer};
    ///
    /// let server = WireframeServer::new(|| WireframeApp::default());
    /// assert!(server.worker_count() >= 1);
    /// ```
    #[must_use]
    pub fn new(factory: F) -> Self {
        let workers = std::thread::available_parallelism().map_or(1, std::num::NonZeroUsize::get);
        Self { factory, workers, on_preamble_success: None, on_preamble_failure: None, ready_tx: None, state: Unbound, _preamble: PhantomData }
    }
}

impl<F, T, S> WireframeServer<F, T, S>
where
    F: Fn() -> WireframeApp + Send + Sync + Clone + 'static,
    T: Preamble,
    S: ServerState,
{
    /// Set the number of worker tasks to spawn for the server.
    ///
    /// # Examples
    ///
    /// ```
    /// use wireframe::{app::WireframeApp, server::WireframeServer};
    ///
    /// let server = WireframeServer::new(|| WireframeApp::default()).workers(4);
    /// assert_eq!(server.worker_count(), 4);
    /// ```
    #[must_use]
    pub fn workers(mut self, count: usize) -> Self {
        self.workers = count.max(1);
        self
    }

    /// Configure a channel used to signal when the server is ready to accept connections.
    ///
    /// # Examples
    ///
    /// ```
    /// use tokio::sync::oneshot;
    /// use wireframe::{app::WireframeApp, server::WireframeServer};
    ///
    /// let (tx, _rx) = oneshot::channel();
    /// let server = WireframeServer::new(|| WireframeApp::default()).ready_signal(tx);
    /// ```
    #[must_use]
    pub fn ready_signal(mut self, tx: oneshot::Sender<()>) -> Self {
        self.ready_tx = Some(tx);
        self
    }

    /// Returns the configured number of worker tasks for the server.
    ///
    /// # Examples
    ///
    /// ```
    /// use wireframe::{app::WireframeApp, server::WireframeServer};
    ///
    /// let server = WireframeServer::new(|| WireframeApp::default()).workers(8);
    /// assert_eq!(server.worker_count(), 8);
    /// ```
    #[inline]
    #[must_use]
    pub const fn worker_count(&self) -> usize { self.workers }


}
