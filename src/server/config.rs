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

#[cfg(test)]
mod tests {
    use std::{
        net::SocketAddr,
        sync::{
            Arc,
            atomic::{AtomicUsize, Ordering},
        },
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
        // Mirror the default worker logic to keep tests aligned with `WireframeServer::new`.
        std::thread::available_parallelism().map_or(1, std::num::NonZeroUsize::get)
    }

    #[rstest]
    fn test_new_server_creation(
        factory: impl Fn() -> WireframeApp + Send + Sync + Clone + 'static,
    ) {
        let server = WireframeServer::new(factory);
        assert!(server.worker_count() >= 1 && server.local_addr().is_none());
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
        let mut server = WireframeServer::new(factory);
        server = server.workers(4);
        assert_eq!(server.worker_count(), 4);
        server = server.workers(100);
        assert_eq!(server.worker_count(), 100);
        assert_eq!(server.workers(0).worker_count(), 1);
    }

    #[rstest]
    fn test_with_preamble_type_conversion(
        factory: impl Fn() -> WireframeApp + Send + Sync + Clone + 'static,
    ) {
        let server = WireframeServer::new(factory).with_preamble::<TestPreamble>();
        assert_eq!(server.worker_count(), expected_default_worker_count());
    }

    #[rstest]
    #[tokio::test]
    async fn test_bind_success(
        factory: impl Fn() -> WireframeApp + Send + Sync + Clone + 'static,
        free_port: SocketAddr,
    ) {
        let local_addr = WireframeServer::new(factory)
            .bind(free_port)
            .expect("Failed to bind")
            .local_addr()
            .expect("local address missing");
        assert_eq!(local_addr.ip(), free_port.ip());
    }

    #[rstest]
    fn test_local_addr_before_bind(
        factory: impl Fn() -> WireframeApp + Send + Sync + Clone + 'static,
    ) {
        assert!(WireframeServer::new(factory).local_addr().is_none());
    }

    #[rstest]
    #[tokio::test]
    async fn test_local_addr_after_bind(
        factory: impl Fn() -> WireframeApp + Send + Sync + Clone + 'static,
        free_port: SocketAddr,
    ) {
        let local_addr = bind_server(factory, free_port).local_addr().unwrap();
        assert_eq!(local_addr.ip(), free_port.ip());
    }

    #[rstest]
    #[case("success")]
    #[case("failure")]
    #[tokio::test]
    async fn test_preamble_callback_registration(
        factory: impl Fn() -> WireframeApp + Send + Sync + Clone + 'static,
        #[case] callback_type: &str,
    ) {
        let counter = Arc::new(AtomicUsize::new(0));
        let c = counter.clone();

        let server = server_with_preamble(factory);
        let server = match callback_type {
            "success" => server.on_preamble_decode_success(move |_p: &TestPreamble, _| {
                let c = c.clone();
                Box::pin(async move {
                    c.fetch_add(1, Ordering::SeqCst);
                    Ok(())
                })
            }),
            "failure" => server.on_preamble_decode_failure(move |_err: &DecodeError| {
                c.fetch_add(1, Ordering::SeqCst);
            }),
            _ => panic!("Invalid callback type"),
        };

        assert_eq!(counter.load(Ordering::SeqCst), 0);
        match callback_type {
            "success" => assert!(server.on_preamble_success.is_some()),
            "failure" => assert!(server.on_preamble_failure.is_some()),
            _ => unreachable!(),
        }
    }

    #[rstest]
    #[tokio::test]
    async fn test_method_chaining(
        factory: impl Fn() -> WireframeApp + Send + Sync + Clone + 'static,
        free_port: SocketAddr,
    ) {
        let callback_invoked = Arc::new(AtomicUsize::new(0));
        let counter = callback_invoked.clone();
        let server = WireframeServer::new(factory)
            .workers(2)
            .with_preamble::<TestPreamble>()
            .on_preamble_decode_success(move |_p: &TestPreamble, _| {
                let c = counter.clone();
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
        assert_eq!(callback_invoked.load(Ordering::SeqCst), 0);
    }

    #[rstest]
    #[tokio::test]
    async fn test_server_configuration_persistence(
        factory: impl Fn() -> WireframeApp + Send + Sync + Clone + 'static,
        free_port: SocketAddr,
    ) {
        let server = WireframeServer::new(factory)
            .workers(5)
            .bind(free_port)
            .expect("Failed to bind");
        assert_eq!(server.worker_count(), 5);
        assert!(server.local_addr().is_some());
    }

    #[rstest]
    fn test_extreme_worker_counts(
        factory: impl Fn() -> WireframeApp + Send + Sync + Clone + 'static,
    ) {
        let mut server = WireframeServer::new(factory);
        server = server.workers(usize::MAX);
        assert_eq!(server.worker_count(), usize::MAX);
        assert_eq!(server.workers(0).worker_count(), 1);
    }

    #[rstest]
    #[tokio::test]
    async fn test_bind_to_multiple_addresses(
        factory: impl Fn() -> WireframeApp + Send + Sync + Clone + 'static,
        free_port: SocketAddr,
    ) {
        let listener2 =
            std::net::TcpListener::bind(SocketAddr::new(std::net::Ipv4Addr::LOCALHOST.into(), 0))
                .expect("failed to bind second listener");
        let addr2 = listener2
            .local_addr()
            .expect("failed to get second listener address");
        drop(listener2);

        let server = WireframeServer::new(factory);
        let server = server
            .bind(free_port)
            .expect("Failed to bind first address");
        let first = server.local_addr().expect("first bound address missing");
        let server = server.bind(addr2).expect("Failed to bind second address");
        let second = server.local_addr().expect("second bound address missing");
        assert_ne!(first.port(), second.port());
        assert_eq!(second.ip(), addr2.ip());
    }
}
