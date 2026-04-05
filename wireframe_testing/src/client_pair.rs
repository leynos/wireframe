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

/// Active server task and connected client.
///
/// Fields are `Option` so they can be individually extracted during shutdown
/// while keeping the `Running` struct in place until all awaits complete.
struct Running {
    client: Option<WireframeClient<BincodeSerializer, RewindStream<tokio::net::TcpStream>, ()>>,
    shutdown_tx: Option<oneshot::Sender<()>>,
    handle: Option<JoinHandle<Result<(), wireframe::server::ServerError>>>,
}

/// Connected server and client pair for in-process integration tests.
///
/// Holds a running [`WireframeServer`] task and a connected
/// [`WireframeClient`]. The server listens on a real loopback TCP socket so
/// that compatibility assertions exercise the full network path.
///
/// Call [`shutdown`](Self::shutdown) to stop the server gracefully. If the
/// pair is dropped without an explicit shutdown the [`Drop`] implementation
/// sends the shutdown signal and waits up to 100 milliseconds for the server
/// task to complete before aborting it as a safety net.
///
/// [`WireframeServer`]: wireframe::server::WireframeServer
/// [`WireframeClient`]: wireframe::client::WireframeClient
pub struct WireframePair {
    addr: SocketAddr,
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

    /// Shut down the server gracefully and await its task.
    ///
    /// Calls the client's close method to run teardown hooks, sends the
    /// shutdown signal to the server, and joins the server task. Keeps
    /// `Running` in `self` until all awaits complete, so if the future is
    /// cancelled the `Drop` impl can still clean up.
    ///
    /// # Errors
    ///
    /// Returns a [`TestError`] if the server task panicked, was cancelled,
    /// or returned a [`ServerError`](wireframe::server::ServerError).
    pub async fn shutdown(&mut self) -> TestResult<()> {
        let Some(running) = self.running.as_mut() else {
            return Ok(());
        };

        // Take the client and close it.
        if let Some(client) = running.client.take() {
            client.close().await;
        }

        // Send the shutdown signal.
        if let Some(shutdown_tx) = running.shutdown_tx.take() {
            let _ = shutdown_tx.send(());
        }

        // Await the server task through a mutable borrow.
        let Some(handle) = running.handle.as_mut() else {
            return Ok(());
        };

        let result = match handle.await {
            Err(join_err) => Err(TestError::Msg(format!(
                "server task join error: {join_err}"
            ))),
            Ok(Err(server_err)) => Err(TestError::Msg(format!("server error: {server_err}"))),
            Ok(Ok(())) => Ok(()),
        };

        // Now that the await is complete, clear Running.
        self.running = None;

        result
    }
}

impl Drop for WireframePair {
    fn drop(&mut self) {
        if let Some(running) = self.running.take() {
            if let Some(shutdown_tx) = running.shutdown_tx {
                let _ = shutdown_tx.send(());
            }
            if let Some(handle) = running.handle {
                spawn_bounded_shutdown(handle, std::time::Duration::from_millis(100));
            }
        }
    }
}

/// Spawn a task to await server shutdown with a bounded timeout.
///
/// Tries to join the server task handle with a timeout. If running inside a
/// tokio runtime, spawns an async task that races the join against the
/// timeout, aborting only if the timeout expires. If not in a runtime,
/// immediately aborts the task.
///
/// This gives the server task a chance to run `tracker.close()` and
/// `tracker.wait().await` for spawned connection tasks before being
/// force-aborted.
fn spawn_bounded_shutdown(
    mut handle: JoinHandle<Result<(), wireframe::server::ServerError>>,
    timeout: std::time::Duration,
) {
    if let Ok(runtime) = tokio::runtime::Handle::try_current() {
        runtime.spawn(async move {
            tokio::select! {
                _ = &mut handle => {
                    // Task completed within timeout.
                }
                _ = tokio::time::sleep(timeout) => {
                    // Timeout expired, abort the task.
                    handle.abort();
                }
            }
        });
    } else {
        // Not in a runtime, force-abort immediately.
        handle.abort();
    }
}

/// RAII guard for a pending server task that aborts on drop.
///
/// Ensures that if the server task is not successfully handed off to a
/// [`WireframePair`], it will be cleanly shut down via
/// [`spawn_bounded_shutdown`] instead of leaking.
struct PendingServer(
    Option<(
        oneshot::Sender<()>,
        JoinHandle<Result<(), wireframe::server::ServerError>>,
    )>,
);

impl PendingServer {
    /// Take the shutdown channel and handle out of the guard, consuming it.
    fn take(
        &mut self,
    ) -> Option<(
        oneshot::Sender<()>,
        JoinHandle<Result<(), wireframe::server::ServerError>>,
    )> {
        self.0.take()
    }
}

impl Drop for PendingServer {
    fn drop(&mut self) {
        if let Some((shutdown_tx, handle)) = self.0.take() {
            let _ = shutdown_tx.send(());
            spawn_bounded_shutdown(handle, std::time::Duration::from_millis(100));
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
/// If the client connection fails, the server task is torn down before the
/// error is returned so that no orphaned tasks or bound listeners leak into
/// subsequent tests.
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

    // Guard the server task so it's cleaned up if we panic or return early.
    let mut pending = PendingServer(Some((shutdown_tx, handle)));

    // Wait for the server to signal ready.
    ready_rx
        .await
        .map_err(|_| TestError::Msg("server did not signal ready".into()))?;

    // Connect the client while the guard is still held.
    let builder = configure_client(WireframeClientBuilder::new());
    let client = builder.connect(addr).await?;

    // Now that the client is connected, take the pending server out of the guard.
    let (shutdown_tx, handle) = pending.take().expect("pending server already taken");

    Ok(WireframePair {
        addr,
        running: Some(Running {
            client: Some(client),
            shutdown_tx: Some(shutdown_tx),
            handle: Some(handle),
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
