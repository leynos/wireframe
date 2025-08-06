//! Configuration utilities for [`WireframeServer`].
//!
//! Provides a fluent builder for configuring `WireframeServer` instances.
//! The builder exposes worker count tuning, preamble callbacks, ready-signal
//! configuration, and TCP binding. The server holds an optional listener so it
//! may be constructed unbound and later bound via [`bind`](WireframeServer::bind).

use core::marker::PhantomData;
use std::{
    io,
    net::{SocketAddr, TcpListener as StdTcpListener},
    sync::Arc,
};

use bincode::error::DecodeError;
use futures::future::BoxFuture;
use tokio::{net::TcpListener, sync::oneshot};

use super::WireframeServer;
use crate::{app::WireframeApp, preamble::Preamble};

#[cfg(test)]
mod tests;

impl<F> WireframeServer<F, ()>
where
    F: Fn() -> WireframeApp + Send + Sync + Clone + 'static,
{
    /// Create a new `WireframeServer` from the given application factory.
    ///
    /// The worker count defaults to the number of available CPU cores (or 1 if
    /// this cannot be determined). The TCP listener is unset; call
    /// [`bind`](Self::bind) before running the server.
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
            listener: None,
            _preamble: PhantomData,
        }
    }
}

impl<F, T> WireframeServer<F, T>
where
    F: Fn() -> WireframeApp + Send + Sync + Clone + 'static,
    T: Preamble,
{
    /// Converts the server to use a custom preamble type for incoming
    /// connections.
    ///
    /// Calling this method drops any previously configured preamble decode
    /// callbacks.
    ///
    /// # Examples
    ///
    /// ```
    /// use bincode::{Decode, Encode};
    /// use wireframe::{app::WireframeApp, preamble::Preamble, server::WireframeServer};
    ///
    /// #[derive(Encode, Decode)]
    /// struct MyPreamble;
    /// impl Preamble for MyPreamble {}
    ///
    /// let server = WireframeServer::new(|| WireframeApp::default()).with_preamble::<MyPreamble>();
    /// ```
    #[must_use]
    pub fn with_preamble<P>(self) -> WireframeServer<F, P>
    where
        P: Preamble,
    {
        WireframeServer {
            factory: self.factory,
            workers: self.workers,
            on_preamble_success: None,
            on_preamble_failure: None,
            ready_tx: None,
            listener: self.listener,
            _preamble: PhantomData,
        }
    }

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

    /// Returns the bound address, or `None` if not yet bound.
    #[must_use]
    pub fn local_addr(&self) -> Option<SocketAddr> {
        self.listener.as_ref().and_then(|l| l.local_addr().ok())
    }

    /// Bind to a fresh address.
    ///
    /// # Errors
    /// Returns an `io::Error` if binding or configuring the listener fails.
    pub fn bind(self, addr: SocketAddr) -> io::Result<Self> {
        let std = StdTcpListener::bind(addr)?;
        self.bind_listener(std)
    }

    /// Bind to an existing `StdTcpListener`.
    ///
    /// # Errors
    /// Returns an `io::Error` if configuring the listener fails.
    pub fn bind_listener(mut self, std: StdTcpListener) -> io::Result<Self> {
        std.set_nonblocking(true)?;
        let tokio = TcpListener::from_std(std)?;
        self.listener = Some(Arc::new(tokio));
        Ok(self)
    }
}
