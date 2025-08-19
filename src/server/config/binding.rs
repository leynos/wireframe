//! Binding configuration for [`WireframeServer`].

use core::marker::PhantomData;
use std::{
    net::{SocketAddr, TcpListener as StdTcpListener},
    sync::Arc,
};

use tokio::{net::TcpListener, sync::oneshot};

use super::{ServerState, Unbound, WireframeServer};
use crate::{
    app::WireframeApp,
    preamble::Preamble,
    server::{BackoffConfig, Bound, PreambleErrorHandler, PreambleHandler, ServerError},
};

/// Configuration for binding an existing [`StdTcpListener`] to a server.
///
/// Grouping these fields avoids long parameter lists when constructing a
/// [`WireframeServer`].
struct BindConfig<F, T> {
    factory: F,
    workers: usize,
    on_preamble_success: Option<PreambleHandler<T>>,
    on_preamble_failure: Option<PreambleErrorHandler>,
    ready_tx: Option<oneshot::Sender<()>>,
    backoff_config: BackoffConfig,
    _preamble: PhantomData<T>,
}

fn bind_std_listener<F, T>(
    config: BindConfig<F, T>,
    std: StdTcpListener,
) -> Result<WireframeServer<F, T, Bound>, ServerError>
where
    F: Fn() -> WireframeApp + Send + Sync + Clone + 'static,
    T: Preamble,
{
    std.set_nonblocking(true).map_err(ServerError::Bind)?;
    let tokio = TcpListener::from_std(std).map_err(ServerError::Bind)?;
    Ok(WireframeServer {
        factory: config.factory,
        workers: config.workers,
        on_preamble_success: config.on_preamble_success,
        on_preamble_failure: config.on_preamble_failure,
        ready_tx: config.ready_tx,
        backoff_config: config.backoff_config,
        state: Bound {
            listener: Arc::new(tokio),
        },
        _preamble: config._preamble,
    })
}

impl<F, T, S> WireframeServer<F, T, S>
where
    F: Fn() -> WireframeApp + Send + Sync + Clone + 'static,
    T: Preamble,
    S: ServerState,
{
    fn bind_to_listener(
        self,
        std: StdTcpListener,
    ) -> Result<WireframeServer<F, T, Bound>, ServerError> {
        let config = BindConfig {
            factory: self.factory,
            workers: self.workers,
            on_preamble_success: self.on_preamble_success,
            on_preamble_failure: self.on_preamble_failure,
            ready_tx: self.ready_tx,
            backoff_config: self.backoff_config,
            _preamble: self._preamble,
        };
        bind_std_listener(config, std)
    }
}

impl<F, T> WireframeServer<F, T, Unbound>
where
    F: Fn() -> WireframeApp + Send + Sync + Clone + 'static,
    T: Preamble,
{
    /// Return `None` as the server is not bound.
    ///
    /// # Examples
    ///
    /// ```
    /// use wireframe::{app::WireframeApp, server::WireframeServer};
    ///
    /// assert!(
    ///     WireframeServer::new(|| WireframeApp::default())
    ///         .local_addr()
    ///         .is_none()
    /// );
    /// ```
    #[must_use]
    pub const fn local_addr(&self) -> Option<SocketAddr> { None }

    /// Bind to a fresh address.
    ///
    /// # Examples
    ///
    /// ```
    /// use std::net::{Ipv4Addr, SocketAddr};
    ///
    /// use wireframe::{app::WireframeApp, server::WireframeServer};
    ///
    /// let addr = SocketAddr::from((Ipv4Addr::LOCALHOST, 0));
    /// let server = WireframeServer::new(|| WireframeApp::default())
    ///     .bind(addr)
    ///     .expect("bind failed");
    /// assert!(server.local_addr().is_some());
    /// ```
    ///
    /// # Errors
    /// Returns a [`ServerError`] if binding or configuring the listener fails.
    pub fn bind(self, addr: SocketAddr) -> Result<WireframeServer<F, T, Bound>, ServerError> {
        let std = StdTcpListener::bind(addr).map_err(ServerError::Bind)?;
        self.bind_existing_listener(std)
    }

    /// Bind to an existing `StdTcpListener`.
    ///
    /// # Examples
    ///
    /// ```
    /// use std::net::{Ipv4Addr, SocketAddr, TcpListener as StdTcpListener};
    ///
    /// use wireframe::{app::WireframeApp, server::WireframeServer};
    ///
    /// let std = StdTcpListener::bind(SocketAddr::from((Ipv4Addr::LOCALHOST, 0))).unwrap();
    /// let server = WireframeServer::new(|| WireframeApp::default())
    ///     .bind_existing_listener(std)
    ///     .expect("bind failed");
    /// assert!(server.local_addr().is_some());
    /// ```
    ///
    /// # Errors
    /// Returns a [`ServerError`] if configuring the listener fails.
    pub fn bind_existing_listener(
        self,
        std: StdTcpListener,
    ) -> Result<WireframeServer<F, T, Bound>, ServerError> {
        self.bind_to_listener(std)
    }
}

impl<F, T> WireframeServer<F, T, Bound>
where
    F: Fn() -> WireframeApp + Send + Sync + Clone + 'static,
    T: Preamble,
{
    /// Returns the bound address, or `None` if retrieving it fails.
    ///
    /// # Examples
    ///
    /// ```
    /// use std::net::SocketAddr;
    ///
    /// use wireframe::{app::WireframeApp, server::WireframeServer};
    ///
    /// let addr: SocketAddr = "127.0.0.1:0".parse().unwrap();
    /// let server = WireframeServer::new(|| WireframeApp::default())
    ///     .bind(addr)
    ///     .expect("bind failed");
    /// assert!(server.local_addr().is_some());
    /// ```
    #[must_use]
    pub fn local_addr(&self) -> Option<SocketAddr> { self.state.listener.local_addr().ok() }

    /// Rebind to a fresh address.
    ///
    /// # Examples
    ///
    /// ```
    /// use std::net::{Ipv4Addr, SocketAddr};
    ///
    /// use wireframe::{app::WireframeApp, server::WireframeServer};
    ///
    /// let addr = SocketAddr::from((Ipv4Addr::LOCALHOST, 0));
    /// let server = WireframeServer::new(|| WireframeApp::default())
    ///     .bind(addr)
    ///     .expect("bind failed");
    /// let addr2 = SocketAddr::from((Ipv4Addr::LOCALHOST, 0));
    /// let server = server.bind(addr2).expect("rebind failed");
    /// assert!(server.local_addr().is_some());
    /// ```
    ///
    /// # Errors
    /// Returns a [`ServerError`] if binding or configuring the listener fails.
    pub fn bind(self, addr: SocketAddr) -> Result<Self, ServerError> {
        let std = StdTcpListener::bind(addr).map_err(ServerError::Bind)?;
        self.bind_existing_listener(std)
    }

    /// Rebind using an existing `StdTcpListener`.
    ///
    /// # Examples
    ///
    /// ```
    /// use std::net::{Ipv4Addr, SocketAddr, TcpListener as StdTcpListener};
    ///
    /// use wireframe::{app::WireframeApp, server::WireframeServer};
    ///
    /// let addr = SocketAddr::from((Ipv4Addr::LOCALHOST, 0));
    /// let server = WireframeServer::new(|| WireframeApp::default())
    ///     .bind(addr)
    ///     .expect("bind failed");
    /// let std = StdTcpListener::bind(SocketAddr::from((Ipv4Addr::LOCALHOST, 0))).unwrap();
    /// let server = server.bind_existing_listener(std).expect("rebind failed");
    /// assert!(server.local_addr().is_some());
    /// ```
    ///
    /// # Errors
    /// Returns a [`ServerError`] if configuring the listener fails.
    pub fn bind_existing_listener(self, std: StdTcpListener) -> Result<Self, ServerError> {
        self.bind_to_listener(std)
    }
}
