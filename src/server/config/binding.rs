//! Listener binding for [`WireframeServer`].

use std::{
    net::{SocketAddr, TcpListener as StdTcpListener},
    sync::Arc,
};

use tokio::net::TcpListener;

use super::{ServerState, Unbound, WireframeServer};
use crate::{
    app::Packet,
    codec::FrameCodec,
    preamble::Preamble,
    serializer::Serializer,
    server::{AppFactory, Bound, ServerError},
};

/// Trait alias for wireframe factory functions.
trait WireframeFactory<Ser, Ctx, E, Codec>: AppFactory<Ser, Ctx, E, Codec>
where
    Ser: Serializer + Send + Sync,
    Ctx: Send + 'static,
    E: Packet,
    Codec: FrameCodec,
{
}
impl<F, Ser, Ctx, E, Codec> WireframeFactory<Ser, Ctx, E, Codec> for F
where
    F: AppFactory<Ser, Ctx, E, Codec>,
    Ser: Serializer + Send + Sync,
    Ctx: Send + 'static,
    E: Packet,
    Codec: FrameCodec,
{
}

/// Helper trait alias for wireframe preambles
trait WireframePreamble: Preamble {}
impl<T> WireframePreamble for T where T: Preamble {}

type BoundServer<F, T, Ser, Ctx, E, Codec> = WireframeServer<F, T, Bound, Ser, Ctx, E, Codec>;

/// Blanket impl uses private trait aliases; suppress visibility lint
#[expect(
    private_bounds,
    reason = "helper trait aliases are module-private by design"
)]
impl<F, T, S, Ser, Ctx, E, Codec> WireframeServer<F, T, S, Ser, Ctx, E, Codec>
where
    F: WireframeFactory<Ser, Ctx, E, Codec>,
    T: WireframePreamble,
    S: ServerState,
    Ser: Serializer + Send + Sync,
    Ctx: Send + 'static,
    E: Packet,
    Codec: FrameCodec,
{
    fn bind_to_listener(
        self,
        std_listener: StdTcpListener,
    ) -> Result<BoundServer<F, T, Ser, Ctx, E, Codec>, ServerError> {
        let WireframeServer {
            factory,
            workers,
            on_preamble_success,
            on_preamble_failure,
            ready_tx,
            backoff_config,
            preamble_timeout,
            _app: app_marker,
            _preamble: preamble_marker,
            ..
        } = self;

        std_listener
            .set_nonblocking(true)
            .map_err(ServerError::Bind)?;
        let tokio_listener = TcpListener::from_std(std_listener).map_err(ServerError::Bind)?;

        Ok(WireframeServer {
            factory,
            workers,
            on_preamble_success,
            on_preamble_failure,
            ready_tx,
            backoff_config,
            preamble_timeout,
            state: Bound {
                listener: Arc::new(tokio_listener),
            },
            _app: app_marker,
            _preamble: preamble_marker,
        })
    }
}

/// Blanket impl uses private trait aliases; suppress visibility lint
#[expect(
    private_bounds,
    reason = "helper trait aliases are module-private by design"
)]
impl<F, T, Ser, Ctx, E, Codec> WireframeServer<F, T, Unbound, Ser, Ctx, E, Codec>
where
    F: WireframeFactory<Ser, Ctx, E, Codec>,
    T: WireframePreamble,
    Ser: Serializer + Send + Sync,
    Ctx: Send + 'static,
    E: Packet,
    Codec: FrameCodec,
{
    /// Return `None` as the server is not bound.
    ///
    /// # Examples
    ///
    /// ```
    /// use wireframe::{app::WireframeApp, server::WireframeServer};
    ///
    /// assert!(
    ///     WireframeServer::new(|| -> WireframeApp { WireframeApp::default() })
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
    /// ```no_run
    /// use std::net::{Ipv4Addr, SocketAddr};
    ///
    /// use wireframe::{app::WireframeApp, server::WireframeServer};
    ///
    /// let addr = SocketAddr::from((Ipv4Addr::LOCALHOST, 0));
    /// let server = WireframeServer::new(|| -> WireframeApp { WireframeApp::default() })
    ///     .bind(addr)
    ///     .expect("bind failed");
    /// assert!(server.local_addr().is_some());
    /// ```
    ///
    /// # Errors
    /// Returns a [`ServerError`] if binding or configuring the listener fails.
    pub fn bind(
        self,
        addr: SocketAddr,
    ) -> Result<BoundServer<F, T, Ser, Ctx, E, Codec>, ServerError> {
        let std_listener = StdTcpListener::bind(addr).map_err(ServerError::Bind)?;
        self.bind_existing_listener(std_listener)
    }

    /// Bind to an existing `StdTcpListener`.
    ///
    /// # Examples
    ///
    /// ```no_run
    /// use std::net::{Ipv4Addr, SocketAddr, TcpListener as StdTcpListener};
    ///
    /// use wireframe::{app::WireframeApp, server::WireframeServer};
    ///
    /// let std_listener = StdTcpListener::bind(SocketAddr::from((Ipv4Addr::LOCALHOST, 0))).unwrap();
    /// let server = WireframeServer::new(|| -> WireframeApp { WireframeApp::default() })
    ///     .bind_existing_listener(std_listener)
    ///     .expect("bind failed");
    /// assert!(server.local_addr().is_some());
    /// ```
    ///
    /// # Errors
    /// Returns a [`ServerError`] if configuring the listener fails.
    pub fn bind_existing_listener(
        self,
        std_listener: StdTcpListener,
    ) -> Result<BoundServer<F, T, Ser, Ctx, E, Codec>, ServerError> {
        self.bind_to_listener(std_listener)
    }
}

/// Blanket impl uses private trait aliases; suppress visibility lint
#[expect(
    private_bounds,
    reason = "helper trait aliases are module-private by design"
)]
impl<F, T, Ser, Ctx, E, Codec> WireframeServer<F, T, Bound, Ser, Ctx, E, Codec>
where
    F: WireframeFactory<Ser, Ctx, E, Codec>,
    T: WireframePreamble,
    Ser: Serializer + Send + Sync,
    Ctx: Send + 'static,
    E: Packet,
    Codec: FrameCodec,
{
    /// Returns the bound address, or `None` if retrieving it fails.
    ///
    /// # Examples
    ///
    /// ```no_run
    /// use std::net::SocketAddr;
    ///
    /// use wireframe::{app::WireframeApp, server::WireframeServer};
    ///
    /// let addr: SocketAddr = "127.0.0.1:0".parse().unwrap();
    /// let server = WireframeServer::new(|| -> WireframeApp { WireframeApp::default() })
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
    /// ```no_run
    /// use std::net::{Ipv4Addr, SocketAddr};
    ///
    /// use wireframe::{app::WireframeApp, server::WireframeServer};
    ///
    /// let addr = SocketAddr::from((Ipv4Addr::LOCALHOST, 0));
    /// let server = WireframeServer::new(|| -> WireframeApp { WireframeApp::default() })
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
        let std_listener = StdTcpListener::bind(addr).map_err(ServerError::Bind)?;
        self.bind_existing_listener(std_listener)
    }

    /// Rebind using an existing `StdTcpListener`.
    ///
    /// # Examples
    ///
    /// ```no_run
    /// use std::net::{Ipv4Addr, SocketAddr, TcpListener as StdTcpListener};
    ///
    /// use wireframe::{app::WireframeApp, server::WireframeServer};
    ///
    /// let addr = SocketAddr::from((Ipv4Addr::LOCALHOST, 0));
    /// let server = WireframeServer::new(|| -> WireframeApp { WireframeApp::default() })
    ///     .bind(addr)
    ///     .expect("bind failed");
    /// let std_listener = StdTcpListener::bind(SocketAddr::from((Ipv4Addr::LOCALHOST, 0))).unwrap();
    /// let server = server
    ///     .bind_existing_listener(std_listener)
    ///     .expect("rebind failed");
    /// assert!(server.local_addr().is_some());
    /// ```
    ///
    /// # Errors
    /// Returns a [`ServerError`] if configuring the listener fails.
    pub fn bind_existing_listener(self, std_listener: StdTcpListener) -> Result<Self, ServerError> {
        self.bind_to_listener(std_listener)
    }
}
