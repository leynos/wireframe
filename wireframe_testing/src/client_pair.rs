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
    server::{AppFactory, ServerError, WireframeServer},
};

use crate::{
    TestError,
    TestResult,
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
/// [`RunningWireframeServer`](crate::RunningWireframeServer).
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

/// Re-wrap a startup failure as the pair's historical message shape.
///
/// Only [`TestError::Server`] is re-wrapped: an address or binding failure is
/// also a server error, and Table 1 in the plan keeps those typed. Callers
/// therefore see this mapping applied to the second half of the startup
/// sequence only, never to the bind.
fn map_pair_startup_error(error: TestError) -> TestError {
    match error {
        TestError::Server(server_error) => {
            TestError::Msg(format!("server failed to start: {server_error}"))
        }
        other => other,
    }
}

/// Re-wrap the handle's shutdown failure as the pair's historical shape.
///
/// The pair predates `ServerShutdown`, so a supervisor panic arrived as a
/// `JoinError` and any other server failure as a plain server error. The
/// handle reports both as `TestError::Server`; the source-less abnormal
/// termination is the panic class.
fn map_pair_shutdown_error(error: TestError) -> TestError {
    match error {
        TestError::Server(ServerError::AbnormalTermination { source: None, .. }) => {
            TestError::Msg(format!("server task join error: {error}"))
        }
        TestError::Server(_) => TestError::Msg(format!("server error: {error}")),
        other => other,
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
    // The pair's historical waits are unbounded; keep them that way so no new
    // failure mode appears for existing callers.
    let options = ServerHarnessOptions::default()
        .with_readiness_timeout(None)
        .with_shutdown_timeout(None);

    let listener = unused_listener()?;
    let server = WireframeServer::new(app_factory).workers(1);
    let (bound, addr, ready_rx) = bind_with_readiness(server, listener)?;
    let running_server = start_bound(bound, addr, ready_rx, options)
        .await
        .map_err(map_pair_startup_error)?;

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
mod tests {
    //! Unit tests for the pair's error mapping.
    //!
    //! Each case pins the variant and message prefix a caller observes, which
    //! is the part of the pair's contract the delegation must not change.

    use std::io;

    use wireframe::server::ServerError;

    use super::{map_pair_shutdown_error, map_pair_startup_error};
    use crate::TestError;

    /// The factory error the server reports when startup fails.
    fn factory_failure() -> ServerError {
        ServerError::FactoryBuild(Box::new(io::Error::other("boom")))
    }

    #[test]
    fn startup_mapping_rewraps_only_server_errors() {
        let mapped = map_pair_startup_error(TestError::Server(factory_failure()));
        match mapped {
            TestError::Msg(message) => assert!(
                message.starts_with("server failed to start: "),
                "unexpected message: {message}"
            ),
            other => panic!("a server error must be re-wrapped, got {other:?}"),
        }

        let io_error = TestError::Io(io::Error::other("reserve"));
        assert!(
            matches!(map_pair_startup_error(io_error), TestError::Io(_)),
            "an IO failure must keep its typed variant"
        );
        let message = TestError::Msg("custom".to_owned());
        assert!(
            matches!(map_pair_startup_error(message), TestError::Msg(text) if text == "custom"),
            "a message must pass through unchanged"
        );
    }

    #[test]
    fn shutdown_mapping_separates_the_panic_class() {
        let panicked = TestError::Server(ServerError::AbnormalTermination {
            message: "panic".to_owned(),
            source: None,
        });
        match map_pair_shutdown_error(panicked) {
            TestError::Msg(message) => assert!(
                message.starts_with("server task join error: "),
                "unexpected message: {message}"
            ),
            other => panic!("a source-less termination must map to the join error, got {other:?}"),
        }

        let returned = TestError::Server(ServerError::AbnormalTermination {
            message: "returned".to_owned(),
            source: Some(std::sync::Arc::new(factory_failure())),
        });
        match map_pair_shutdown_error(returned) {
            TestError::Msg(message) => assert!(
                message.starts_with("server error: "),
                "unexpected message: {message}"
            ),
            other => panic!("a returned server error must map to server error, got {other:?}"),
        }
    }
}
