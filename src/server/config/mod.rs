//! Configuration utilities for [`WireframeServer`].

use core::marker::PhantomData;
use std::{
    io,
    net::{SocketAddr, TcpListener as StdTcpListener},
    sync::Arc,
};

use bincode::error::DecodeError;
use futures::future::BoxFuture;
use tokio::{net::TcpListener, sync::oneshot};

use super::{Bound, PreambleCallback, PreambleErrorCallback, Unbound, WireframeServer};
use crate::{app::WireframeApp, preamble::Preamble};

#[cfg(test)]
mod tests;

impl<F> WireframeServer<F, (), Unbound>
where
    F: Fn() -> WireframeApp + Send + Sync + Clone + 'static,
{
    /// Create a new `WireframeServer` from the given application factory.
    ///
    /// The worker count defaults to the number of available CPU cores (or 1 if this cannot be
    /// determined). The TCP listener is unset; call [`bind`](Self::bind) before running the
    /// server.
    #[must_use]
    pub fn new(factory: F) -> Self {
        let workers = std::thread::available_parallelism().map_or(1, std::num::NonZeroUsize::get);
        Self {
            factory,
            workers,
            on_preamble_success: None,
            on_preamble_failure: None,
            ready_tx: None,
            state: Unbound,
            _preamble: PhantomData,
        }
    }
}

impl<F, S> WireframeServer<F, (), S>
where
    F: Fn() -> WireframeApp + Send + Sync + Clone + 'static,
{
    /// Converts the server to use a custom preamble type for incoming connections.
    ///
    /// Calling this method drops any previously configured preamble decode callbacks.
    #[must_use]
    pub fn with_preamble<P>(self) -> WireframeServer<F, P, S>
    where
        P: Preamble,
    {
        WireframeServer {
            factory: self.factory,
            workers: self.workers,
            on_preamble_success: None,
            on_preamble_failure: None,
            ready_tx: None,
            state: self.state,
            _preamble: PhantomData,
        }
    }
}

impl<F, T, S> WireframeServer<F, T, S>
where
    F: Fn() -> WireframeApp + Send + Sync + Clone + 'static,
    T: Preamble,
{
    /// Set the number of worker tasks to spawn for the server.
    #[must_use]
    pub fn workers(mut self, count: usize) -> Self {
        self.workers = count.max(1);
        self
    }

    /// Register a callback invoked when the connection preamble decodes successfully.
    #[must_use]
    pub fn on_preamble_decode_success<H>(mut self, handler: H) -> Self
    where
        H: for<'a> Fn(&'a T, &'a mut tokio::net::TcpStream) -> BoxFuture<'a, io::Result<()>>
            + Send
            + Sync
            + 'static,
    {
        self.on_preamble_success = Some(Arc::new(handler));
        self
    }

    /// Register a callback invoked when the connection preamble fails to decode.
    #[must_use]
    pub fn on_preamble_decode_failure<H>(mut self, handler: H) -> Self
    where
        H: Fn(&DecodeError) + Send + Sync + 'static,
    {
        self.on_preamble_failure = Some(Arc::new(handler));
        self
    }

    /// Configure a channel used to signal when the server is ready to accept connections.
    #[must_use]
    pub fn ready_signal(mut self, tx: oneshot::Sender<()>) -> Self {
        self.ready_tx = Some(tx);
        self
    }

    /// Returns the configured number of worker tasks for the server.
    #[inline]
    #[must_use]
    pub const fn worker_count(&self) -> usize { self.workers }

    /// Delegate binding to [`bind_std_listener`] after extracting fields.
    ///
    /// The public `bind` and `bind_listener` methods merely prepare the
    /// [`StdTcpListener`] before calling this helper.
    fn bind_with_std_listener(
        self,
        std_listener: StdTcpListener,
    ) -> io::Result<WireframeServer<F, T, Bound>> {
        let Self {
            factory,
            workers,
            on_preamble_success,
            on_preamble_failure,
            ready_tx,
            ..
        } = self;
        bind_std_listener(
            factory,
            workers,
            on_preamble_success,
            on_preamble_failure,
            ready_tx,
            std_listener,
        )
    }
}

impl<F, T> WireframeServer<F, T, Unbound>
where
    F: Fn() -> WireframeApp + Send + Sync + Clone + 'static,
    T: Preamble,
{
    /// Get the socket address the server is bound to, if available.
    #[must_use]
    pub const fn local_addr(&self) -> Option<SocketAddr> { None }

    /// Bind the server to the given address and create a listener.
    ///
    /// # Errors
    /// Returns an `io::Error` if binding or configuring the listener fails.
    pub fn bind(self, addr: SocketAddr) -> io::Result<WireframeServer<F, T, Bound>> {
        let std_listener = StdTcpListener::bind(addr)?;
        self.bind_with_std_listener(std_listener)
    }

    /// Bind the server to an existing standard TCP listener.
    ///
    /// # Errors
    /// Returns an [`io::Error`] if configuring the listener fails.
    pub fn bind_listener(
        self,
        listener: StdTcpListener,
    ) -> io::Result<WireframeServer<F, T, Bound>> {
        self.bind_with_std_listener(listener)
    }
}

impl<F, T> WireframeServer<F, T, Bound>
where
    F: Fn() -> WireframeApp + Send + Sync + Clone + 'static,
    T: Preamble,
{
    /// Get the socket address the server is bound to.
    #[must_use]
    pub fn local_addr(&self) -> Option<SocketAddr> { self.state.listener.local_addr().ok() }

    /// Rebind the server to a new address.
    ///
    /// # Errors
    /// Returns an `io::Error` if binding or configuring the listener fails.
    pub fn bind(self, addr: SocketAddr) -> io::Result<Self> {
        let std_listener = StdTcpListener::bind(addr)?;
        self.bind_with_std_listener(std_listener)
    }

    /// Rebind the server to an existing standard TCP listener.
    ///
    /// # Errors
    /// Returns an [`io::Error`] if configuring the listener fails.
    pub fn bind_listener(self, listener: StdTcpListener) -> io::Result<Self> {
        self.bind_with_std_listener(listener)
    }
}

#[allow(clippy::too_many_arguments)]
fn bind_std_listener<F, T>(
    factory: F,
    workers: usize,
    on_preamble_success: Option<PreambleCallback<T>>,
    on_preamble_failure: Option<PreambleErrorCallback>,
    ready_tx: Option<oneshot::Sender<()>>,
    std_listener: StdTcpListener,
) -> io::Result<WireframeServer<F, T, Bound>>
where
    F: Fn() -> WireframeApp + Send + Sync + Clone + 'static,
    T: Preamble,
{
    std_listener.set_nonblocking(true)?;
    let listener = TcpListener::from_std(std_listener)?;
    Ok(WireframeServer {
        factory,
        workers,
        on_preamble_success,
        on_preamble_failure,
        ready_tx,
        state: Bound {
            listener: Arc::new(listener),
        },
        _preamble: PhantomData,
    })
}
