//! In-process server and client pair harness.
//!
//! This module provides [`WireframePair`], a reusable test harness that starts
//! a real [`WireframeServer`] bound to a loopback TCP listener and connects a
//! [`WireframeClient`] inside one test process. Both sides communicate over a
//! real loopback socket, keeping compatibility checks honest while remaining
//! fast and deterministic.
//!
//! The pair delegates the server lifecycle to
//! [`server_harness`](crate::server_harness) and keeps only the default-client
//! façade: a configured server in, a connected client out. Its startup and
//! shutdown failures are re-wrapped as [`TestError::Msg`] where the historical
//! contract did so, so existing callers observe the same variants.
//!
//! [`WireframeServer`]: wireframe::server::WireframeServer
//! [`WireframeClient`]: wireframe::client::WireframeClient
//!
//! # Examples
//!
//! ```rust,no_run
//! use wireframe::app::WireframeApp;
//! use wireframe_testing::{TestResult, client_pair::spawn_wireframe_pair};
//!
//! # async fn example() -> TestResult<()> {
//! let mut pair = spawn_wireframe_pair(
//!     || -> WireframeApp { WireframeApp::default() },
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

use wireframe::{
    app::Packet,
    client::{WireframeClient, WireframeClientBuilder},
    codec::FrameCodec,
    rewind_stream::RewindStream,
    serializer::BincodeSerializer,
    server::{AppFactory, WireframeServer},
};

use crate::{
    TestError,
    TestResult,
    client_pair_errors::{map_pair_shutdown_error, map_pair_startup_error},
    integration_helpers::unused_listener,
    server_harness::{
        RunningWireframeServer,
        ServerHarnessOptions,
        bind_with_readiness,
        start_bound,
    },
};

/// Active server handle and connected client.
///
/// Fields are `Option` so they can be individually extracted during shutdown
/// while keeping the `Running` struct in place until all awaits complete.
struct Running {
    /// Connected client retained until graceful teardown closes its hooks.
    client: Option<WireframeClient<BincodeSerializer, RewindStream<tokio::net::TcpStream>, ()>>,
    /// Harness handle owning the running server.
    server: RunningWireframeServer,
}

/// Connected server and client pair for in-process integration tests.
///
/// Holds a running [`WireframeServer`] and a connected [`WireframeClient`].
/// The server listens on a real loopback TCP socket so that compatibility
/// assertions exercise the full network path.
///
/// Call [`shutdown`](Self::shutdown) to stop the server gracefully. If the
/// pair is dropped without an explicit shutdown, its server handle is dropped,
/// which requests shutdown synchronously; the listener is then released once
/// the accept loops exit, exactly as for a bare
/// [`RunningWireframeServer`].
///
/// [`WireframeServer`]: wireframe::server::WireframeServer
/// [`WireframeClient`]: wireframe::client::WireframeClient
pub struct WireframePair {
    /// Loopback endpoint reserved for the paired client and server.
    addr: SocketAddr,
    /// Live resources, removed only after shutdown completes.
    running: Option<Running>,
}

impl std::fmt::Debug for WireframePair {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("WireframePair")
            .field("addr", &self.addr)
            .field("running", &self.running.as_ref().map(|_| ".."))
            .finish()
    }
}

impl WireframePair {
    /// Borrow the connected client mutably for request/response operations.
    ///
    /// Streaming responses borrow the client exclusively, so this method
    /// returns `&mut` to make that borrow visible in calling code.
    ///
    /// # Errors
    ///
    /// Returns a [`TestError`] if called after
    /// [`shutdown`](Self::shutdown) has completed.
    pub fn client_mut(
        &mut self,
    ) -> TestResult<&mut WireframeClient<BincodeSerializer, RewindStream<tokio::net::TcpStream>, ()>>
    {
        self.running
            .as_mut()
            .and_then(|r| r.client.as_mut())
            .ok_or_else(|| TestError::Msg("client_mut called after shutdown".into()))
    }

    /// Return the loopback address the server is bound to.
    #[must_use]
    pub const fn local_addr(&self) -> SocketAddr { self.addr }

    /// Shut down the server gracefully and await its drain.
    ///
    /// Closes the client to run its teardown hooks, then awaits the server
    /// handle's unbounded shutdown so the server's result is always observed.
    ///
    /// # Errors
    ///
    /// Returns a [`TestError`] if the server reported an abnormal termination
    /// or failed to drain.
    pub async fn shutdown(&mut self) -> TestResult<()> {
        let Some(running) = self.running.as_mut() else {
            return Ok(());
        };

        if let Some(client) = running.client.take() {
            client.close().await;
        }

        let result = running
            .server
            .shutdown()
            .await
            .map_err(map_pair_shutdown_error);
        self.running = None;
        result
    }
}

/// Abort a startup task when the future awaiting it is dropped.
struct AbortOnDrop<T>(tokio::task::JoinHandle<T>);

impl<T> Drop for AbortOnDrop<T> {
    fn drop(&mut self) { self.0.abort(); }
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
/// If the client connection fails, the server handle is dropped before the
/// error is returned, so the server is asked to stop and no listener leaks
/// into subsequent tests.
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
///     || -> WireframeApp { WireframeApp::default() },
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
    start_pair(app_factory, configure_client, unused_listener()?).await
}

/// Start a pair on a caller-supplied listener.
///
/// The public helpers reserve their own listener; this core exists so an
/// in-crate test can know the address a cancelled startup must release.
async fn start_pair<F, E, Codec, B>(
    app_factory: F,
    configure_client: B,
    listener: std::net::TcpListener,
) -> TestResult<WireframePair>
where
    F: AppFactory<BincodeSerializer, (), E, Codec>,
    E: Packet,
    Codec: FrameCodec,
    B: FnOnce(
        WireframeClientBuilder<BincodeSerializer, (), ()>,
    ) -> WireframeClientBuilder<BincodeSerializer, (), ()>,
{
    // The pair's historical waits are unbounded; keep them that way so no new
    // failure mode appears for existing callers.
    let options = ServerHarnessOptions::default()
        .with_readiness_timeout(None)
        .with_shutdown_timeout(None);

    let server = WireframeServer::new(app_factory).workers(1);
    let (bound, addr, ready_rx) = bind_with_readiness(server, listener)?;

    // Startup runs in a task, as it did before the delegation, so a panic
    // surfaces as the historical join-error message. The guard aborts the task
    // if this future is cancelled, so a cancelled startup drops the bound
    // server instead of detaching a task that still owns the listener.
    let mut startup = AbortOnDrop(tokio::spawn(start_bound(bound, addr, ready_rx, options)));
    let running_server = match (&mut startup.0).await {
        Err(join_error) => {
            return Err(TestError::Msg(format!(
                "server task failed to start: {join_error}"
            )));
        }
        Ok(result) => result.map_err(map_pair_startup_error)?,
    };

    let builder = configure_client(WireframeClientBuilder::new());
    let client = builder.connect(addr).await?;

    Ok(WireframePair {
        addr,
        running: Some(Running {
            client: Some(client),
            server: running_server,
        }),
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
/// let mut pair =
///     spawn_wireframe_pair_default(|| -> WireframeApp { WireframeApp::default() }).await?;
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

#[cfg(test)]
#[path = "client_pair_cancellation_tests.rs"]
mod startup_cancellation_tests;
