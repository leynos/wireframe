//! Configuration utilities for [`WireframeServer`].
//!
//! Provides a fluent builder for configuring server instances. Worker counts,
//! ready-signal channels and optional callbacks are set here. TCP binding lives
//! in the [`binding`] module; preamble behaviour is customised
//! via [`preamble`]. Servers start [`Unbound`](super::Unbound)
//! and must call [`bind`](super::WireframeServer::bind) or
//! [`bind_existing_listener`](super::WireframeServer::bind_existing_listener)
//! before running. The `run` methods are available only once the server is
//! [`Bound`](super::Bound).

use core::marker::PhantomData;

use tokio::sync::oneshot;

use super::{BackoffConfig, ServerState, Unbound, WireframeServer};
use crate::{app::WireframeApp, preamble::Preamble};

macro_rules! builder_setter {
    ($(#[$meta:meta])* $fn:ident, $field:ident, $arg:ident: $ty:ty => $assign:expr) => {
        $(#[$meta])*
        #[must_use]
        pub fn $fn(mut self, $arg: $ty) -> Self {
            self.$field = $assign;
            self
        }
    };
}

macro_rules! builder_callback {
    ($(#[$meta:meta])* $fn:ident, $field:ident, $($bound:tt)*) => {
        $(#[$meta])*
        #[must_use]
        pub fn $fn<H>(mut self, handler: H) -> Self
        where
            H: $($bound)*,
        {
            self.$field = Some(std::sync::Arc::new(handler));
            self
        }
    };
}

pub mod binding;
pub mod preamble;

impl<F> WireframeServer<F, (), Unbound>
where
    F: Fn() -> WireframeApp + Send + Sync + Clone + 'static,
{
    /// Create a new `WireframeServer` from the given application factory.
    ///
    /// The worker count defaults to the number of available CPU cores (or 1 if
    /// this cannot be determined). The server is initially [`Unbound`]; call
    /// [`bind`](WireframeServer::bind) or
    /// [`bind_existing_listener`](WireframeServer::bind_existing_listener)
    /// (methods provided by the [`binding`] module) before running the server.
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
        Self {
            factory,
            workers,
            on_preamble_success: None,
            on_preamble_failure: None,
            ready_tx: None,
            backoff_config: BackoffConfig::default(),
            state: Unbound,
            _preamble: PhantomData,
        }
    }
}

impl<F, T, S> WireframeServer<F, T, S>
where
    F: Fn() -> WireframeApp + Send + Sync + Clone + 'static,
    T: Preamble,
    S: ServerState,
{
    builder_setter!(
        /// Set the number of worker tasks to spawn for the server.
        ///
        /// A minimum of one worker is enforced.
        ///
        /// # Examples
        ///
        /// ```
        /// use wireframe::{app::WireframeApp, server::WireframeServer};
        ///
        /// let server = WireframeServer::new(|| WireframeApp::default()).workers(4);
        /// assert_eq!(server.worker_count(), 4);
        ///
        /// let server = WireframeServer::new(|| WireframeApp::default()).workers(0);
        /// assert_eq!(server.worker_count(), 1);
        /// ```
        workers, workers, count: usize => count.max(1)
    );

    builder_setter!(
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
        ready_signal, ready_tx, tx: oneshot::Sender<()> => Some(tx)
    );

    builder_setter!(
        /// Configure accept-loop backoff behaviour.
        ///
        /// The supplied configuration is passed to
        /// [`BackoffConfig::normalised`] (`cfg.normalised()`) before being
        /// stored. Normalisation clamps `initial_delay` to at least 1 ms and no
        /// greater than `max_delay`. If `initial_delay` exceeds `max_delay`,
        /// the values are swapped. Normalisation applies any other adjustments
        /// `BackoffConfig::normalised` defines so out-of-range values are
        /// corrected rather than preserved.
        ///
        /// Invariants:
        /// - `initial_delay` must be >= 1 ms
        /// - `initial_delay` must be <= `max_delay`
        accept_backoff, backoff_config, cfg: BackoffConfig => cfg.normalised()
    );

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
