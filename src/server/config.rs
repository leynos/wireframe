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

use super::WireframeServer;
use crate::{app::WireframeApp, preamble::Preamble};

impl<F> WireframeServer<F, ()>
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
            listener: None,
            workers,
            on_preamble_success: None,
            on_preamble_failure: None,
            ready_tx: None,
            _preamble: PhantomData,
        }
    }

    /// Converts the server to use a custom preamble type for incoming connections.
    ///
    /// Calling this method drops any previously configured preamble decode callbacks.
    #[must_use]
    pub fn with_preamble<P>(self) -> WireframeServer<F, P>
    where
        P: Preamble,
    {
        WireframeServer {
            factory: self.factory,
            listener: self.listener,
            workers: self.workers,
            on_preamble_success: None,
            on_preamble_failure: None,
            ready_tx: None,
            _preamble: PhantomData,
        }
    }
}

impl<F, T> WireframeServer<F, T>
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

    /// Get the socket address the server is bound to, if available.
    #[must_use]
    pub fn local_addr(&self) -> Option<SocketAddr> {
        self.listener.as_ref().and_then(|l| l.local_addr().ok())
    }

    /// Bind the server to the given address and create a listener.
    ///
    /// # Errors
    /// Returns an `io::Error` if binding or configuring the listener fails.
    pub fn bind(self, addr: SocketAddr) -> io::Result<Self> {
        let std_listener = StdTcpListener::bind(addr)?;
        self.bind_std_listener(std_listener)
    }

    /// Bind the server to an existing standard TCP listener.
    ///
    /// # Errors
    /// Returns an [`io::Error`] if configuring the listener fails.
    pub fn bind_listener(self, listener: StdTcpListener) -> io::Result<Self> {
        self.bind_std_listener(listener)
    }

    fn bind_std_listener(mut self, std_listener: StdTcpListener) -> io::Result<Self> {
        std_listener.set_nonblocking(true)?;
        let listener = TcpListener::from_std(std_listener)?;
        self.listener = Some(Arc::new(listener));
        Ok(self)
    }
}

#[cfg(test)]
mod tests {
    use std::sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    };

    use rstest::rstest;

    use super::*;
    use crate::server::test_util::{
        TestPreamble,
        bind_server,
        factory,
        free_port,
        server_with_preamble,
    };

    fn expected_default_worker_count() -> usize {
        // Mirror the default worker logic to keep tests aligned with
        // `WireframeServer::new`.
        std::thread::available_parallelism().map_or(1, std::num::NonZeroUsize::get)
    }

    #[rstest]
    fn test_new_server_creation(
        factory: impl Fn() -> WireframeApp + Send + Sync + Clone + 'static,
    ) {
        let server = WireframeServer::new(factory);
        assert!(server.worker_count() >= 1);
        assert!(server.local_addr().is_none());
    }

    #[rstest]
    fn test_new_server_default_worker_count(
        factory: impl Fn() -> WireframeApp + Send + Sync + Clone + 'static,
    ) {
        let server = WireframeServer::new(factory);
        assert_eq!(server.worker_count(), expected_default_worker_count());
    }

    #[rstest]
    fn test_workers_configuration(
        factory: impl Fn() -> WireframeApp + Send + Sync + Clone + 'static,
    ) {
        let server = WireframeServer::new(factory);
        let server = server.workers(4);
        assert_eq!(server.worker_count(), 4);
        let server = server.workers(100);
        assert_eq!(server.worker_count(), 100);
        let server = server.workers(0);
        assert_eq!(server.worker_count(), 1);
    }

    #[rstest]
    fn test_with_preamble_type_conversion(
        factory: impl Fn() -> WireframeApp + Send + Sync + Clone + 'static,
    ) {
        let server = WireframeServer::new(factory);
        let server_with_preamble = server.with_preamble::<TestPreamble>();
        assert_eq!(
            server_with_preamble.worker_count(),
            expected_default_worker_count(),
        );
    }

    #[rstest]
    #[tokio::test]
    async fn test_bind_success(
        factory: impl Fn() -> WireframeApp + Send + Sync + Clone + 'static,
        free_port: std::net::SocketAddr,
    ) {
        let server = WireframeServer::new(factory);
        let server = server.bind(free_port).expect("Failed to bind");
        let local_addr = server.local_addr().expect("local address missing");
        assert_eq!(local_addr.ip(), free_port.ip());
    }

    #[rstest]
    fn test_local_addr_before_bind(
        factory: impl Fn() -> WireframeApp + Send + Sync + Clone + 'static,
    ) {
        let server = WireframeServer::new(factory);
        assert!(server.local_addr().is_none());
    }

    #[rstest]
    #[tokio::test]
    async fn test_local_addr_after_bind(
        factory: impl Fn() -> WireframeApp + Send + Sync + Clone + 'static,
        free_port: std::net::SocketAddr,
    ) {
        let server = bind_server(factory, free_port);
        let local_addr = server.local_addr();
        assert!(local_addr.is_some());
        assert_eq!(local_addr.unwrap().ip(), free_port.ip());
    }

    #[rstest]
    #[tokio::test]
    async fn test_preamble_success_callback(
        factory: impl Fn() -> WireframeApp + Send + Sync + Clone + 'static,
    ) {
        let counter = Arc::new(AtomicUsize::new(0));
        let c = counter.clone();
        let server = server_with_preamble(factory).on_preamble_decode_success(
            move |_p: &TestPreamble, _| {
                let c = c.clone();
                Box::pin(async move {
                    c.fetch_add(1, Ordering::SeqCst);
                    Ok(())
                })
            },
        );
        assert_eq!(counter.load(Ordering::SeqCst), 0);
        assert!(server.on_preamble_success.is_some());
    }

    #[rstest]
    #[tokio::test]
    async fn test_preamble_failure_callback(
        factory: impl Fn() -> WireframeApp + Send + Sync + Clone + 'static,
    ) {
        let counter = Arc::new(AtomicUsize::new(0));
        let c = counter.clone();
        let server =
            server_with_preamble(factory).on_preamble_decode_failure(move |_err: &DecodeError| {
                c.fetch_add(1, Ordering::SeqCst);
            });
        assert_eq!(counter.load(Ordering::SeqCst), 0);
        assert!(server.on_preamble_failure.is_some());
    }

    #[rstest]
    #[tokio::test]
    async fn test_method_chaining(
        factory: impl Fn() -> WireframeApp + Send + Sync + Clone + 'static,
        free_port: std::net::SocketAddr,
    ) {
        let counter = Arc::new(AtomicUsize::new(0));
        let c = counter.clone();
        let server = WireframeServer::new(factory)
            .workers(2)
            .with_preamble::<TestPreamble>()
            .on_preamble_decode_success(move |_p: &TestPreamble, _| {
                let c = c.clone();
                Box::pin(async move {
                    c.fetch_add(1, Ordering::SeqCst);
                    Ok(())
                })
            })
            .on_preamble_decode_failure(|_: &DecodeError| {})
            .bind(free_port)
            .expect("Failed to bind");
        assert_eq!(server.worker_count(), 2);
        assert!(server.local_addr().is_some());
        assert!(server.on_preamble_success.is_some());
        assert!(server.on_preamble_failure.is_some());
    }

    #[rstest]
    #[tokio::test]
    async fn test_server_configuration_persistence(
        factory: impl Fn() -> WireframeApp + Send + Sync + Clone + 'static,
        free_port: std::net::SocketAddr,
    ) {
        let server = WireframeServer::new(factory).workers(5);
        assert_eq!(server.worker_count(), 5);
        let server = server.bind(free_port).expect("Failed to bind");
        assert_eq!(server.worker_count(), 5);
        assert!(server.local_addr().is_some());
    }

    #[rstest]
    fn test_preamble_callbacks_reset_on_type_change(
        factory: impl Fn() -> WireframeApp + Send + Sync + Clone + 'static,
    ) {
        let server = WireframeServer::new(factory)
            .on_preamble_decode_success(|&(), _| Box::pin(async { Ok(()) }))
            .on_preamble_decode_failure(|_: &DecodeError| {});
        assert!(server.on_preamble_success.is_some());
        assert!(server.on_preamble_failure.is_some());
        let server = server.with_preamble::<TestPreamble>();
        assert!(server.on_preamble_success.is_none());
        assert!(server.on_preamble_failure.is_none());
    }

    #[rstest]
    fn test_extreme_worker_counts(
        factory: impl Fn() -> WireframeApp + Send + Sync + Clone + 'static,
    ) {
        let server = WireframeServer::new(factory);
        let server = server.workers(usize::MAX);
        assert_eq!(server.worker_count(), usize::MAX);
        let server = server.workers(0);
        assert_eq!(server.worker_count(), 1);
    }

    #[rstest]
    #[tokio::test]
    async fn test_bind_to_multiple_addresses(
        factory: impl Fn() -> WireframeApp + Send + Sync + Clone + 'static,
        free_port: std::net::SocketAddr,
    ) {
        let server = WireframeServer::new(factory);
        let addr1 = free_port;
        let addr2 = {
            let addr = SocketAddr::new(std::net::Ipv4Addr::LOCALHOST.into(), 0);
            let listener =
                std::net::TcpListener::bind(addr).expect("failed to bind second listener");
            listener
                .local_addr()
                .expect("failed to get second listener address")
        };
        let server = server.bind(addr1).expect("Failed to bind first address");
        let first = server.local_addr().expect("first bound address missing");
        let server = server.bind(addr2).expect("Failed to bind second address");
        let second = server.local_addr().expect("second bound address missing");
        assert_ne!(first.port(), second.port());
        assert_eq!(second.ip(), addr2.ip());
    }
}
