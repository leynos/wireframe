//! In-process server and client pair harness.
//!
//! This module provides [`WireframePair`], a reusable test harness that starts
//! a real [`WireframeServer`] bound to a loopback TCP listener and connects a
//! [`WireframeClient`] inside one test process. Both sides communicate over a
//! real loopback socket, keeping compatibility checks honest while remaining
//! fast and deterministic.
//!
//! The harness owns the server lifecycle and exposes the connected client for
//! direct use in test assertions. It offers an explicit [`shutdown`] path and
//! a defensive [`Drop`] implementation that aborts orphaned server tasks.
//!
//! [`WireframeServer`]: wireframe::server::WireframeServer
//! [`WireframeClient`]: wireframe::client::WireframeClient
//! [`shutdown`]: WireframePair::shutdown
//!
//! # Examples
//!
//! ```rust,no_run
//! use wireframe::app::WireframeApp;
//! use wireframe_testing::{TestResult, client_pair::spawn_wireframe_pair};
//!
//! # async fn example() -> TestResult<()> {
//! let mut pair = spawn_wireframe_pair(
//!     || WireframeApp::default(),
//!     |builder| builder.max_frame_length(2048),
//! )
//! .await?;
//!
//! // Use the client for request/response assertions.
//! let addr = pair.local_addr();
//! assert!(addr.port() > 0);
//!
//! pair.shutdown().await?;
//! # Ok(())
//! # }
//! ```

use std::net::SocketAddr;

use tokio::{sync::oneshot, task::JoinHandle};
use wireframe::{
    app::Packet,
    client::{WireframeClient, WireframeClientBuilder},
    codec::FrameCodec,
    rewind_stream::RewindStream,
    serializer::BincodeSerializer,
    server::{AppFactory, WireframeServer},
};

use crate::{TestError, TestResult, integration_helpers::unused_listener};

/// Connected server and client pair for in-process integration tests.
///
/// Holds a running [`WireframeServer`] task and a connected
/// [`WireframeClient`]. The server listens on a real loopback TCP socket so
/// that compatibility assertions exercise the full network path.
///
/// Call [`shutdown`](Self::shutdown) to stop the server gracefully. If the
/// pair is dropped without an explicit shutdown the [`Drop`] implementation
/// sends the shutdown signal and aborts the server task after a brief delay
/// as a safety net.
///
/// [`WireframeServer`]: wireframe::server::WireframeServer
/// [`WireframeClient`]: wireframe::client::WireframeClient
pub struct WireframePair {
    client: Option<WireframeClient<BincodeSerializer, RewindStream<tokio::net::TcpStream>, ()>>,
    addr: SocketAddr,
    shutdown_tx: Option<oneshot::Sender<()>>,
    handle: Option<JoinHandle<Result<(), wireframe::server::ServerError>>>,
}

impl std::fmt::Debug for WireframePair {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("WireframePair")
            .field("addr", &self.addr)
            .field("client", &self.client.as_ref().map(|_| ".."))
            .field("shutdown_tx", &self.shutdown_tx.is_some())
            .field("handle", &self.handle.as_ref().map(|_| ".."))
            .finish()
    }
}

impl WireframePair {
    /// Borrow the connected client mutably for request/response operations.
    ///
    /// Streaming responses borrow the client exclusively, so this method
    /// returns `&mut` to make that borrow visible in calling code.
    ///
    /// # Panics
    ///
    /// Panics if called after [`shutdown`](Self::shutdown) has completed.
    #[expect(
        clippy::expect_used,
        reason = "intentional panic for post-shutdown misuse"
    )]
    pub fn client_mut(
        &mut self,
    ) -> &mut WireframeClient<BincodeSerializer, RewindStream<tokio::net::TcpStream>, ()> {
        self.client
            .as_mut()
            .expect("client_mut called after shutdown")
    }

    /// Return the loopback address the server is bound to.
    #[must_use]
    pub const fn local_addr(&self) -> SocketAddr { self.addr }

    /// Shut down the server gracefully and await its task.
    ///
    /// Sends the shutdown signal, drops the client, and joins the server
    /// task. Any error from the server task is surfaced as a [`TestError`].
    ///
    /// # Errors
    ///
    /// Returns a [`TestError`] if the server task panicked, was cancelled,
    /// or returned a [`ServerError`](wireframe::server::ServerError).
    pub async fn shutdown(&mut self) -> TestResult<()> {
        // Drop the client first so the server sees the disconnect.
        self.client.take();

        if let Some(tx) = self.shutdown_tx.take() {
            let _ = tx.send(());
        }

        if let Some(handle) = self.handle.take() {
            handle
                .await
                .map_err(|e| TestError::Msg(format!("server task join error: {e}")))??;
        }

        Ok(())
    }
}

impl Drop for WireframePair {
    fn drop(&mut self) {
        if let Some(tx) = self.shutdown_tx.take() {
            let _ = tx.send(());
        }
        if let Some(handle) = self.handle.take() {
            let abort_handle = handle.abort_handle();
            std::thread::spawn(move || {
                std::thread::sleep(std::time::Duration::from_secs(5));
                abort_handle.abort();
            });
        }
    }
}

/// Start a server and connect a client, returning a [`WireframePair`].
///
/// This is the primary entry point for in-process pair tests. It reserves a
/// loopback TCP listener, spawns a [`WireframeServer`] with the supplied
/// `app_factory`, connects a [`WireframeClient`] configured through
/// `configure_client`, and returns the pair handle.
///
/// The `app_factory` closure is called by the server for every accepted
/// connection. The `configure_client` closure receives a default
/// [`WireframeClientBuilder`] and returns the configured builder — use this
/// to set frame length, hooks, or other client-side options.
///
/// [`WireframeServer`]: wireframe::server::WireframeServer
/// [`WireframeClient`]: wireframe::client::WireframeClient
/// [`WireframeClientBuilder`]: wireframe::client::WireframeClientBuilder
///
/// # Errors
///
/// Returns a [`TestError`] if binding the listener, starting the server, or
/// connecting the client fails.
///
/// # Examples
///
/// ```rust,no_run
/// use wireframe::app::WireframeApp;
/// use wireframe_testing::{TestResult, client_pair::spawn_wireframe_pair};
///
/// # async fn example() -> TestResult<()> {
/// let mut pair = spawn_wireframe_pair(
///     || WireframeApp::default(),
///     |builder| builder.max_frame_length(2048),
/// )
/// .await?;
///
/// let addr = pair.local_addr();
/// pair.shutdown().await?;
/// # Ok(())
/// # }
/// ```
pub async fn spawn_wireframe_pair<F, E, Codec, B>(
    app_factory: F,
    configure_client: B,
) -> TestResult<WireframePair>
where
    F: AppFactory<BincodeSerializer, (), E, Codec>,
    E: Packet,
    Codec: FrameCodec,
    B: FnOnce(
        WireframeClientBuilder<BincodeSerializer, (), ()>,
    ) -> WireframeClientBuilder<BincodeSerializer, (), ()>,
{
    let listener = unused_listener()?;
    let server = WireframeServer::new(app_factory)
        .workers(1)
        .bind_existing_listener(listener)?;
    let addr = server
        .local_addr()
        .ok_or("server did not report a bound address")?;

    let (shutdown_tx, shutdown_rx) = oneshot::channel();
    let (ready_tx, ready_rx) = oneshot::channel();

    let handle = tokio::spawn(async move {
        server
            .ready_signal(ready_tx)
            .run_with_shutdown(async {
                let _ = shutdown_rx.await;
            })
            .await
    });

    ready_rx
        .await
        .map_err(|_| TestError::Msg("server did not signal ready".into()))?;

    let builder = configure_client(WireframeClientBuilder::new());
    let client = builder.connect(addr).await?;

    Ok(WireframePair {
        client: Some(client),
        addr,
        shutdown_tx: Some(shutdown_tx),
        handle: Some(handle),
    })
}

/// Start a server and connect a client using default client settings.
///
/// Convenience wrapper around [`spawn_wireframe_pair`] that uses a default
/// [`WireframeClientBuilder`] without additional configuration.
///
/// [`WireframeClientBuilder`]: wireframe::client::WireframeClientBuilder
///
/// # Errors
///
/// Returns a [`TestError`] if binding the listener, starting the server, or
/// connecting the client fails.
///
/// # Examples
///
/// ```rust,no_run
/// use wireframe::app::WireframeApp;
/// use wireframe_testing::{TestResult, client_pair::spawn_wireframe_pair_default};
///
/// # async fn example() -> TestResult<()> {
/// let mut pair = spawn_wireframe_pair_default(|| WireframeApp::default()).await?;
/// pair.shutdown().await?;
/// # Ok(())
/// # }
/// ```
pub async fn spawn_wireframe_pair_default<F, E, Codec>(app_factory: F) -> TestResult<WireframePair>
where
    F: AppFactory<BincodeSerializer, (), E, Codec>,
    E: Packet,
    Codec: FrameCodec,
{
    spawn_wireframe_pair(app_factory, |builder| builder).await
}
