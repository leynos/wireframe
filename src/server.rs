//! Tokio-based server for `WireframeApp` instances.
//!
//! `WireframeServer` spawns worker tasks to accept TCP connections,
//! optionally decoding a connection preamble before handing the
//! stream to the application.

use core::marker::PhantomData;
use std::{
    io,
    net::{SocketAddr, TcpListener as StdTcpListener},
    sync::Arc,
};

use bincode::error::DecodeError;
use futures::future::BoxFuture;

/// Callback invoked when a connection preamble decodes successfully.
///
/// The callback may perform asynchronous I/O on the provided stream before the
/// connection is handed off to [`WireframeApp`].
pub type PreambleCallback<T> = Arc<
    dyn for<'a> Fn(&'a T, &'a mut tokio::net::TcpStream) -> BoxFuture<'a, io::Result<()>>
        + Send
        + Sync,
>;

/// Callback invoked when decoding a connection preamble fails.
pub type PreambleErrorCallback = Arc<dyn Fn(&DecodeError) + Send + Sync>;
use tokio::{net::TcpListener, sync::oneshot};
use tokio_util::{sync::CancellationToken, task::TaskTracker};

use crate::{app::WireframeApp, preamble::Preamble};
mod worker;

#[cfg(test)]
pub use worker::worker_task;
/// Tokio-based server for `WireframeApp` instances.
///
/// `WireframeServer` spawns a worker task per thread. Each worker
/// receives its own `WireframeApp` from the provided factory
/// closure. The server listens for a shutdown signal using
/// `tokio::signal::ctrl_c` and notifies all workers to stop
/// accepting new connections.
pub struct WireframeServer<F, T = ()>
where
    F: Fn() -> WireframeApp + Send + Sync + Clone + 'static,
    // `Preamble` covers types implementing `BorrowDecode` for any lifetime,
    // enabling decoding of borrowed data without external context.
    T: Preamble,
{
    factory: F,
    listener: Option<Arc<TcpListener>>,
    workers: usize,
    on_preamble_success: Option<PreambleCallback<T>>,
    on_preamble_failure: Option<PreambleErrorCallback>,
    /// Channel used to notify when the server is ready.
    ///
    /// # Thread Safety
    /// This sender is `Send` and may be moved between threads safely.
    ///
    /// # Single-use Semantics
    /// A `oneshot::Sender` can transmit only one readiness notification. After
    /// sending, the sender is consumed and cannot be reused.
    ///
    /// # Implications
    /// Because only one notification may be sent, a new `ready_tx` must be
    /// provided each time the server is started.
    ready_tx: Option<oneshot::Sender<()>>,
    _preamble: PhantomData<T>,
}

impl<F> WireframeServer<F, ()>
where
    F: Fn() -> WireframeApp + Send + Sync + Clone + 'static,
{
    /// Create a new `WireframeServer` from the given application factory.
    ///
    /// The worker count defaults to the number of available CPU cores (or 1 if this cannot be
    /// determined). The TCP listener is unset; call [`bind`](Self::bind) before running the
    /// server.
    ///
    /// # Examples
    ///
    /// ```no_run
    /// use wireframe::{app::WireframeApp, server::WireframeServer};
    ///
    /// let server = WireframeServer::new(|| WireframeApp::default());
    /// assert!(server.worker_count() >= 1);
    /// ```
    #[must_use]
    pub fn new(factory: F) -> Self {
        // Ensure at least one worker is always configured. While
        // `available_parallelism` cannot return zero, defensive programming
        // protects against unexpected platform behaviour.
        let workers = std::thread::available_parallelism()
            .map_or(1, std::num::NonZeroUsize::get)
            .max(1);
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
    /// Calling this method will drop any previously configured preamble decode callbacks. Use it
    /// before registering preamble handlers if you wish to retain them.
    ///
    /// # Type Parameters
    ///
    /// * `T` – The type to decode as the connection preamble; must implement `bincode::Decode<()>`,
    ///   `Send`, and `'static`.
    ///
    /// # Returns
    ///
    /// A new `WireframeServer` instance configured to decode preambles of type `T`.
    ///
    /// # Examples
    ///
    /// ```no_run
    /// # use wireframe::server::WireframeServer;
    /// # use wireframe::app::WireframeApp;
    /// # let factory = || WireframeApp::new().expect("Failed to initialise app");
    /// #[derive(bincode::Decode)]
    /// # struct MyPreamble;
    /// let server = WireframeServer::new(factory).with_preamble::<MyPreamble>();
    /// ```
    #[must_use]
    pub fn with_preamble<P>(self) -> WireframeServer<F, P>
    where
        // New preamble types must satisfy the `Preamble` bound.
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
    // The preamble type must satisfy the `Preamble` bound.
    T: Preamble,
{
    /// Set the number of worker tasks to spawn for the server.
    ///
    /// The count is clamped to at least one so a worker is always
    /// present. Returns a new server instance with the updated value.
    ///
    /// # Examples
    ///
    /// ```no_run
    /// use wireframe::{app::WireframeApp, server::WireframeServer};
    ///
    /// let factory = || WireframeApp::new().expect("Failed to initialise app");
    /// let server = WireframeServer::new(factory).workers(4);
    /// assert_eq!(server.worker_count(), 4);
    /// let server = server.workers(0);
    /// assert_eq!(server.worker_count(), 1);
    /// ```
    #[must_use]
    pub fn workers(mut self, count: usize) -> Self {
        self.workers = count.max(1);
        self
    }

    /// Register a callback invoked when the connection preamble
    /// decodes successfully.
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

    /// Configure a channel used to signal when the server is ready to accept
    /// connections.
    ///
    /// The provided `oneshot::Sender` is consumed after the first signal. Use a
    /// fresh sender for each server run.
    #[must_use]
    pub fn ready_signal(mut self, tx: oneshot::Sender<()>) -> Self {
        self.ready_tx = Some(tx);
        self
    }

    /// Returns the configured number of worker tasks for the server.
    ///
    /// # Examples
    ///
    /// ```no_run
    /// use wireframe::{app::WireframeApp, server::WireframeServer};
    ///
    /// let factory = || WireframeApp::new().expect("Failed to initialise app");
    /// let server = WireframeServer::new(factory);
    /// assert!(server.worker_count() >= 1);
    /// ```
    #[inline]
    #[must_use]
    pub const fn worker_count(&self) -> usize { self.workers }

    /// Get the socket address the server is bound to, if available.
    #[must_use]
    pub fn local_addr(&self) -> Option<SocketAddr> {
        self.listener.as_ref().and_then(|l| l.local_addr().ok())
    }
    #[doc(hidden)]
    pub fn has_preamble_success(&self) -> bool { self.on_preamble_success.is_some() }
    #[doc(hidden)]
    pub fn has_preamble_failure(&self) -> bool { self.on_preamble_failure.is_some() }

    /// Bind the server to the given address and create a listener.
    ///
    /// # Errors
    ///
    /// Binds the server to the specified socket address and prepares it for accepting TCP
    /// connections.
    ///
    /// Returns an error if binding to the address or configuring the listener fails.
    ///
    /// # Arguments
    ///
    /// * `addr` - The socket address to bind the server to.
    ///
    /// # Returns
    ///
    /// An updated server instance with the listener configured, or an `io::Error` if binding fails.
    ///
    /// # Examples
    ///
    /// ```no_run
    /// use std::net::SocketAddr;
    ///
    /// use wireframe::{app::WireframeApp, server::WireframeServer};
    ///
    /// let factory = || WireframeApp::new().expect("Failed to initialise app");
    /// let server = WireframeServer::new(factory);
    /// let addr: SocketAddr = "127.0.0.1:8080".parse().expect("Failed to parse address");
    /// let server = server.bind(addr).expect("Failed to bind address");
    /// ```
    pub fn bind(mut self, addr: SocketAddr) -> io::Result<Self> {
        let std_listener = StdTcpListener::bind(addr)?;
        std_listener.set_nonblocking(true)?;
        let listener = TcpListener::from_std(std_listener)?;
        self.listener = Some(Arc::new(listener));
        Ok(self)
    }

    /// Run the server until a shutdown signal is received.
    ///
    /// Each worker accepts connections concurrently and would
    /// process them using its `WireframeApp`. Connection handling
    /// logic is not yet implemented.
    ///
    /// # Errors
    ///
    /// Returns an [`io::Error`] if accepting a connection fails.
    ///
    /// # Panics
    ///
    /// Runs the server, accepting TCP connections concurrently until shutdown.
    ///
    /// Spawns the configured number of worker tasks, each accepting incoming connections using a
    /// shared listener and a separate `WireframeApp` instance. The server listens for a Ctrl+C
    /// signal to initiate graceful shutdown, signalling all workers to stop accepting new
    /// connections. Waits for all worker tasks to complete before returning.
    ///
    /// # Panics
    ///
    /// Panics if called before `bind` has been invoked.
    ///
    /// # Returns
    ///
    /// Returns `Ok(())` when the server shuts down gracefully, or an `io::Error` if accepting
    /// connections fails during runtime.
    ///
    /// # Examples
    ///
    /// ```no_run
    /// use std::net::SocketAddr;
    ///
    /// use wireframe::{app::WireframeApp, server::WireframeServer};
    /// async fn run_server() -> std::io::Result<()> {
    ///     let factory = || WireframeApp::new().expect("Failed to initialise app");
    ///     let addr = "127.0.0.1:8080"
    ///         .parse::<SocketAddr>()
    ///         .expect("Failed to parse address");
    ///     let server = WireframeServer::new(factory).workers(4).bind(addr)?;
    ///     server.run().await
    /// }
    /// ```
    pub async fn run(self) -> io::Result<()> {
        self.run_with_shutdown(async {
            let _ = tokio::signal::ctrl_c().await;
        })
        .await
    }

    /// Run the server until the `shutdown` future resolves.
    ///
    /// # Errors
    ///
    /// Returns an [`io::Error`] if accepting a connection fails during
    /// runtime.
    ///
    /// # Panics
    ///
    /// Panics if [`bind`](Self::bind) was not called beforehand.
    pub async fn run_with_shutdown<S>(self, shutdown: S) -> io::Result<()>
    where
        S: futures::Future<Output = ()> + Send,
    {
        let listener = self.listener.expect("`bind` must be called before `run`");
        if let Some(tx) = self.ready_tx {
            let result = tx.send(());
            if result.is_err() {
                tracing::warn!("Failed to send readiness signal: receiver dropped");
            }
        }

        let shutdown_token = CancellationToken::new();
        let tracker = TaskTracker::new();

        for _ in 0..self.workers {
            let listener = Arc::clone(&listener);
            let factory = self.factory.clone();
            let on_success = self.on_preamble_success.clone();
            let on_failure = self.on_preamble_failure.clone();
            let token = shutdown_token.clone();
            let t = tracker.clone();
            tracker.spawn(worker::worker_task(
                listener, factory, on_success, on_failure, token, t,
            ));
        }

        tokio::select! {
            () = shutdown => shutdown_token.cancel(),
            () = tracker.wait() => {}
        }

        tracker.close();
        tracker.wait().await;
        Ok(())
    }
}
