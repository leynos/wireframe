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
use tokio::{
    net::TcpListener,
    sync::oneshot,
    time::{Duration, sleep},
};
use tokio_util::{sync::CancellationToken, task::TaskTracker};

use crate::{
    app::WireframeApp,
    preamble::{Preamble, read_preamble},
    rewind_stream::RewindStream,
};

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
            tracker.spawn(worker_task(
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

/// Spawn a task to process a single TCP connection, logging and discarding any
/// panics from the task.
fn spawn_connection_task<F, T>(
    stream: tokio::net::TcpStream,
    factory: F,
    on_success: Option<PreambleCallback<T>>,
    on_failure: Option<PreambleErrorCallback>,
    tracker: &TaskTracker,
) where
    F: Fn() -> WireframeApp + Send + Sync + Clone + 'static,
    T: Preamble,
{
    let peer_addr = match stream.peer_addr() {
        Ok(addr) => Some(addr),
        Err(e) => {
            tracing::warn!(error = %e, "Failed to retrieve peer address");
            None
        }
    };
    tracker.spawn(async move {
        use futures::FutureExt as _;
        let fut =
            std::panic::AssertUnwindSafe(process_stream(stream, factory, on_success, on_failure))
                .catch_unwind();

        if let Err(panic) = fut.await {
            let panic_msg = panic
                .downcast_ref::<&str>()
                .copied()
                .or_else(|| panic.downcast_ref::<String>().map(String::as_str))
                .unwrap_or("<non-string panic>");
            tracing::error!(panic = %panic_msg, ?peer_addr, "connection task panicked");
        }
    });
}

/// Accept incoming connections until `shutdown` is triggered, retrying on
/// errors with exponential backoff.
async fn accept_loop<F, T>(
    listener: Arc<TcpListener>,
    factory: F,
    on_success: Option<PreambleCallback<T>>,
    on_failure: Option<PreambleErrorCallback>,
    shutdown: CancellationToken,
    tracker: TaskTracker,
) where
    F: Fn() -> WireframeApp + Send + Sync + Clone + 'static,
    T: Preamble,
{
    let mut delay = Duration::from_millis(10);
    loop {
        tokio::select! {
            biased;

            () = shutdown.cancelled() => break,

            res = listener.accept() => match res {
                Ok((stream, _)) => {
                    spawn_connection_task(
                        stream,
                        factory.clone(),
                        on_success.clone(),
                        on_failure.clone(),
                        &tracker,
                    );
                    delay = Duration::from_millis(10);
                }
                Err(e) => {
                    let local_addr = listener.local_addr().ok();
                    tracing::warn!(error = ?e, ?local_addr, "accept error");
                    sleep(delay).await;
                    delay = (delay * 2).min(Duration::from_secs(1));
                }
            },
        }
    }
}

/// Worker task that delegates connection acceptance to `accept_loop`.
///
/// This function serves as an entry point for worker tasks, passing all parameters
/// to `accept_loop` which handles the actual connection acceptance and processing.
async fn worker_task<F, T>(
    listener: Arc<TcpListener>,
    factory: F,
    on_success: Option<PreambleCallback<T>>,
    on_failure: Option<PreambleErrorCallback>,
    shutdown: CancellationToken,
    tracker: TaskTracker,
) where
    F: Fn() -> WireframeApp + Send + Sync + Clone + 'static,
    // `Preamble` ensures `T` supports borrowed decoding.
    T: Preamble,
{
    accept_loop(listener, factory, on_success, on_failure, shutdown, tracker).await;
}

/// Processes an incoming TCP stream by decoding a preamble and dispatching the connection to a
/// `WireframeApp`.
///
/// Attempts to asynchronously decode a preamble of type `T` from the provided stream. If decoding
/// succeeds, invokes the optional success handler, wraps the stream to include any leftover bytes,
/// and passes it to a new `WireframeApp` instance for connection handling. If decoding fails,
/// invokes the optional failure handler and closes the connection.
///
/// # Type Parameters
///
/// - `F`: A factory closure that produces `WireframeApp` instances.
/// - `T`: The preamble type, which must support borrowed decoding via the `Preamble` trait.
///
/// # Examples
///
/// ```no_run
/// # use std::sync::Arc;
/// # use tokio::net::TcpStream;
/// # use wireframe::app::WireframeApp;
/// # async fn example() {
/// let stream: TcpStream = unimplemented!();
/// let factory = || WireframeApp::new();
/// // process_stream::<_, ()>(stream, factory, None, None).await;
/// # }
/// ```
async fn process_stream<F, T>(
    mut stream: tokio::net::TcpStream,
    factory: F,
    on_success: Option<PreambleCallback<T>>,
    on_failure: Option<PreambleErrorCallback>,
) where
    F: Fn() -> WireframeApp + Send + Sync + 'static,
    // `Preamble` ensures `T` supports borrowed decoding.
    T: Preamble,
{
    let peer_addr = stream.peer_addr().ok();
    match read_preamble::<_, T>(&mut stream).await {
        Ok((preamble, leftover)) => {
            if let Some(handler) = on_success.as_ref()
                && let Err(e) = handler(&preamble, &mut stream).await
            {
                tracing::error!(error = ?e, ?peer_addr, "preamble callback error");
            }
            let stream = RewindStream::new(leftover, stream);
            // Hand the connection to the application for processing.
            // We already run `process_stream` inside a task, so spawning again
            // only adds overhead.
            let app = (factory)();
            app.handle_connection(stream).await;
        }
        Err(err) => {
            if let Some(handler) = on_failure.as_ref() {
                handler(&err);
            }
            // drop stream on failure
        }
    }
}

#[cfg(test)]
mod tests {
    use std::{
        net::{Ipv4Addr, SocketAddr},
        sync::{
            Arc,
            atomic::{AtomicUsize, Ordering},
        },
    };

    use bincode::{Decode, Encode};
    use rstest::{fixture, rstest};
    use tokio::{
        net::{TcpListener, TcpStream},
        sync::oneshot,
        time::{Duration, timeout},
    };
    use tokio_util::{sync::CancellationToken, task::TaskTracker};
    use tracing_test::traced_test;

    use super::*;

    #[derive(Debug, Clone, PartialEq, Encode, Decode)]
    struct TestPreamble {
        id: u32,
        message: String,
    }

    /// Test helper preamble carrying no data.
    #[derive(Debug, Clone, PartialEq, Encode, Decode)]
    #[expect(
        dead_code,
        reason = "used only in doctest to illustrate an empty preamble"
    )]
    struct EmptyPreamble;

    #[fixture]
    fn factory() -> impl Fn() -> WireframeApp + Send + Sync + Clone + 'static {
        || WireframeApp::default()
    }

    #[fixture]
    fn free_port() -> SocketAddr {
        let addr = SocketAddr::new(Ipv4Addr::LOCALHOST.into(), 0);
        let listener = std::net::TcpListener::bind(addr).unwrap();
        listener.local_addr().unwrap()
    }

    fn bind_server<F>(factory: F, addr: SocketAddr) -> WireframeServer<F>
    where
        F: Fn() -> WireframeApp + Send + Sync + Clone + 'static,
    {
        WireframeServer::new(factory)
            .bind(addr)
            .expect("Failed to bind")
    }

    fn server_with_preamble<F>(factory: F) -> WireframeServer<F, TestPreamble>
    where
        F: Fn() -> WireframeApp + Send + Sync + Clone + 'static,
    {
        WireframeServer::new(factory).with_preamble::<TestPreamble>()
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
        let expected_workers = std::thread::available_parallelism()
            .map_or(1, std::num::NonZeroUsize::get)
            .max(1);
        assert_eq!(server.worker_count(), expected_workers);
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
            std::thread::available_parallelism()
                .map_or(1, std::num::NonZeroUsize::get)
                .max(1)
        );
    }

    #[rstest]
    #[tokio::test]
    async fn test_bind_success(
        factory: impl Fn() -> WireframeApp + Send + Sync + Clone + 'static,
        free_port: SocketAddr,
    ) {
        let server = bind_server(factory, free_port);
        let bound_addr = server.local_addr().unwrap();
        assert_eq!(bound_addr.ip(), free_port.ip());
    }

    #[rstest]
    #[tokio::test]
    async fn test_bind_invalid_address(
        factory: impl Fn() -> WireframeApp + Send + Sync + Clone + 'static,
    ) {
        let server = WireframeServer::new(factory);
        let addr = SocketAddr::new(Ipv4Addr::LOCALHOST.into(), 1);
        let result = server.bind(addr);
        assert!(result.is_ok() || result.is_err());
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
        free_port: SocketAddr,
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
        let callback_counter = Arc::new(AtomicUsize::new(0));
        let counter_clone = callback_counter.clone();

        let server = server_with_preamble(factory).on_preamble_decode_success(
            move |_preamble: &TestPreamble, _| {
                let cnt = counter_clone.clone();
                Box::pin(async move {
                    cnt.fetch_add(1, Ordering::SeqCst);
                    Ok(())
                })
            },
        );

        assert_eq!(callback_counter.load(Ordering::SeqCst), 0);
        assert!(server.on_preamble_success.is_some());
    }

    #[rstest]
    #[tokio::test]
    async fn test_preamble_failure_callback(
        factory: impl Fn() -> WireframeApp + Send + Sync + Clone + 'static,
    ) {
        let callback_counter = Arc::new(AtomicUsize::new(0));
        let counter_clone = callback_counter.clone();

        let server = server_with_preamble(factory).on_preamble_decode_failure(
            move |_error: &DecodeError| {
                counter_clone.fetch_add(1, Ordering::SeqCst);
            },
        );

        assert_eq!(callback_counter.load(Ordering::SeqCst), 0);
        assert!(server.on_preamble_failure.is_some());
    }

    #[rstest]
    #[tokio::test]
    async fn test_method_chaining(
        factory: impl Fn() -> WireframeApp + Send + Sync + Clone + 'static,
        free_port: SocketAddr,
    ) {
        let callback_invoked = Arc::new(AtomicUsize::new(0));
        let counter_clone = callback_invoked.clone();

        let server = WireframeServer::new(factory)
            .workers(2)
            .with_preamble::<TestPreamble>()
            .on_preamble_decode_success(move |_: &TestPreamble, _| {
                let cnt = counter_clone.clone();
                Box::pin(async move {
                    cnt.fetch_add(1, Ordering::SeqCst);
                    Ok(())
                })
            })
            .on_preamble_decode_failure(|err: &DecodeError| {
                tracing::warn!(error = ?err, "Preamble decode failed");
            })
            .bind(free_port)
            .expect("Failed to bind");

        assert_eq!(server.worker_count(), 2);
        assert!(server.local_addr().is_some());
        assert!(server.on_preamble_success.is_some());
        assert!(server.on_preamble_failure.is_some());
    }

    #[rstest]
    #[tokio::test]
    #[should_panic(expected = "`bind` must be called before `run`")]
    async fn test_run_without_bind_panics(
        factory: impl Fn() -> WireframeApp + Send + Sync + Clone + 'static,
    ) {
        let server = WireframeServer::new(factory);
        let _ = timeout(Duration::from_millis(100), server.run()).await;
    }

    #[rstest]
    #[tokio::test]
    #[should_panic(expected = "`bind` must be called before `run`")]
    async fn test_run_with_shutdown_without_bind_panics(
        factory: impl Fn() -> WireframeApp + Send + Sync + Clone + 'static,
    ) {
        let server = WireframeServer::new(factory);
        let shutdown_future = async { tokio::time::sleep(Duration::from_millis(10)).await };
        let _ = timeout(
            Duration::from_millis(100),
            server.run_with_shutdown(shutdown_future),
        )
        .await;
    }

    #[rstest]
    #[tokio::test]
    async fn test_run_with_immediate_shutdown(
        factory: impl Fn() -> WireframeApp + Send + Sync + Clone + 'static,
        free_port: SocketAddr,
    ) {
        let server = WireframeServer::new(factory)
            .workers(1)
            .bind(free_port)
            .expect("Failed to bind");

        let shutdown_future = async {};

        let result = timeout(
            Duration::from_millis(1000),
            server.run_with_shutdown(shutdown_future),
        )
        .await;

        assert!(result.is_ok());
        assert!(result.unwrap().is_ok());
    }

    #[rstest]
    #[tokio::test]
    async fn test_server_graceful_shutdown_with_ctrl_c_simulation(
        factory: impl Fn() -> WireframeApp + Send + Sync + Clone + 'static,
        free_port: SocketAddr,
    ) {
        let server = WireframeServer::new(factory)
            .workers(2)
            .bind(free_port)
            .expect("Failed to bind");

        let shutdown_future = async {
            tokio::time::sleep(Duration::from_millis(50)).await;
        };

        let start = std::time::Instant::now();
        let result = timeout(
            Duration::from_millis(1000),
            server.run_with_shutdown(shutdown_future),
        )
        .await;
        let elapsed = start.elapsed();

        assert!(result.is_ok());
        assert!(result.unwrap().is_ok());
        assert!(elapsed < Duration::from_millis(500));
    }

    #[rstest]
    #[tokio::test]
    async fn test_multiple_worker_creation(
        factory: impl Fn() -> WireframeApp + Send + Sync + Clone + 'static,
        free_port: SocketAddr,
    ) {
        let _ = &factory;
        let call_count = Arc::new(AtomicUsize::new(0));
        let call_count_clone = call_count.clone();

        let factory = move || {
            call_count_clone.fetch_add(1, Ordering::SeqCst);
            WireframeApp::default()
        };

        let server = WireframeServer::new(factory)
            .workers(3)
            .bind(free_port)
            .expect("Failed to bind");

        let shutdown_future = async {
            tokio::time::sleep(Duration::from_millis(10)).await;
        };

        let result = timeout(
            Duration::from_millis(1000),
            server.run_with_shutdown(shutdown_future),
        )
        .await;

        assert!(result.is_ok());
        assert!(result.unwrap().is_ok());
    }

    #[rstest]
    #[tokio::test]
    async fn test_server_configuration_persistence(
        factory: impl Fn() -> WireframeApp + Send + Sync + Clone + 'static,
        free_port: SocketAddr,
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
    #[tokio::test]
    async fn test_accept_loop_shutdown_signal(
        factory: impl Fn() -> WireframeApp + Send + Sync + Clone + 'static,
    ) {
        let token = CancellationToken::new();
        let tracker = TaskTracker::new();
        let listener = Arc::new(TcpListener::bind("127.0.0.1:0").await.unwrap());

        tracker.spawn(accept_loop::<_, ()>(
            listener,
            factory,
            None,
            None,
            token.clone(),
            tracker.clone(),
        ));

        token.cancel();
        tracker.close();

        let result = timeout(Duration::from_millis(100), tracker.wait()).await;
        assert!(result.is_ok());
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
        free_port: SocketAddr,
    ) {
        let server = WireframeServer::new(factory);
        let addr1 = free_port;
        let addr2 = {
            let addr = SocketAddr::new(Ipv4Addr::LOCALHOST.into(), 0);
            let listener = std::net::TcpListener::bind(addr).unwrap();
            listener.local_addr().unwrap()
        };

        let server = server.bind(addr1).expect("Failed to bind first address");
        let first_local_addr = server.local_addr().unwrap();

        let server = server.bind(addr2).expect("Failed to bind second address");
        let second_local_addr = server.local_addr().unwrap();

        assert_ne!(first_local_addr.port(), second_local_addr.port());
        assert_eq!(second_local_addr.ip(), addr2.ip());
    }

    #[test]
    fn test_server_debug_compilation_guard() {
        assert!(cfg!(debug_assertions));
    }

    /// Panics in connection handlers are logged and do not tear down the worker.
    #[rstest]
    #[traced_test]
    #[tokio::test]
    async fn spawn_connection_task_logs_panic(
        factory: impl Fn() -> WireframeApp + Send + Sync + Clone + 'static,
    ) {
        let app_factory = move || {
            factory()
                .on_connection_setup(|| async { panic!("boom") })
                .unwrap()
        };
        let tracker = TaskTracker::new();
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();

        let handle = tokio::spawn({
            let tracker = tracker.clone();
            async move {
                let (stream, _) = listener.accept().await.unwrap();
                spawn_connection_task::<_, ()>(stream, app_factory, None, None, &tracker);
                tracker.close();
                tracker.wait().await;
            }
        });

        let client = TcpStream::connect(addr).await.unwrap();
        let peer_addr = client.local_addr().unwrap();
        client.writable().await.unwrap();
        client.try_write(&[0; 8]).unwrap();
        drop(client);

        handle.await.unwrap();

        tokio::task::yield_now().await;

        logs_assert(|lines: &[&str]| {
            lines
                .iter()
                .find(|line| {
                    line.contains("connection task panicked")
                        && line.contains("panic=boom")
                        && line.contains(&format!("peer_addr=Some({peer_addr})"))
                })
                .map(|_| ())
                .ok_or_else(|| "panic log not found".to_string())
        });
    }

    /// Ensure the server survives panicking connection tasks.
    ///
    /// The test spawns a server with a connection setup callback that
    /// immediately panics. Logs are captured so the panic message and peer
    /// address can be asserted. A first client
    /// connection triggers the panic and writes dummy preamble bytes to ensure
    /// the panic is logged. The client's peer address is captured before
    /// dropping the connection so the error log can be validated. A second
    /// connection verifies the server continues accepting new clients after the
    /// failure. Finally, the logs are scanned for the expected error entry
    /// containing `peer_addr` and `panic=boom`.
    #[rstest]
    #[traced_test]
    #[tokio::test]
    async fn connection_panic_is_caught(
        factory: impl Fn() -> WireframeApp + Send + Sync + Clone + 'static,
    ) {
        let app_factory = move || {
            factory()
                .on_connection_setup(|| async { panic!("boom") })
                .unwrap()
        };
        let server = WireframeServer::new(app_factory)
            .workers(1)
            .bind("127.0.0.1:0".parse().unwrap())
            .expect("bind");
        let addr = server.local_addr().unwrap();

        let (tx, rx) = oneshot::channel();
        let handle = tokio::spawn(async move {
            server
                .run_with_shutdown(async {
                    let _ = rx.await;
                })
                .await
                .unwrap();
        });

        let first = TcpStream::connect(addr)
            .await
            .expect("first connection should succeed");
        let peer_addr = first.local_addr().expect("first connection peer address");
        first.writable().await.unwrap();
        first.try_write(&[0; 8]).unwrap();
        drop(first);
        TcpStream::connect(addr)
            .await
            .expect("second connection should succeed after panic");

        let _ = tx.send(());
        handle.await.unwrap();

        tokio::task::yield_now().await;

        logs_assert(|lines: &[&str]| {
            lines
                .iter()
                .find(|line| {
                    line.contains("connection task panicked")
                        && line.contains("panic=boom")
                        && line.contains(&format!("peer_addr=Some({peer_addr})"))
                })
                .map(|_| ())
                .ok_or_else(|| "panic log not found".to_string())
        });
    }
}
