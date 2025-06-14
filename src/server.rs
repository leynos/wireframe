use std::io;

#[cfg(not(debug_assertions))]
compile_error!(
    "`wireframe` server functionality is experimental and not intended for production use"
);
use std::net::{SocketAddr, TcpListener as StdTcpListener};
use std::sync::Arc;

use tokio::net::TcpListener;
use tokio::sync::broadcast;
use tokio::time::{Duration, sleep};

use core::marker::PhantomData;

use crate::preamble::{Preamble, read_preamble};
use crate::rewind_stream::RewindStream;
use bincode::error::DecodeError;

use crate::app::WireframeApp;

/// Tokio-based server for `WireframeApp` instances.
///
/// `WireframeServer` spawns a worker task per thread. Each worker
/// receives its own `WireframeApp` from the provided factory
/// closure. The server listens for a shutdown signal using
/// `tokio::signal::ctrl_c` and notifies all workers to stop
/// accepting new connections.
#[allow(clippy::type_complexity)]
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
    on_preamble_success: Option<Arc<dyn Fn(&T) + Send + Sync>>,
    on_preamble_failure: Option<Arc<dyn Fn(&DecodeError) + Send + Sync>>,
    _preamble: PhantomData<T>,
}

impl<F> WireframeServer<F, ()>
where
    F: Fn() -> WireframeApp + Send + Sync + Clone + 'static,
{
    /// Create a new `WireframeServer` from the given application factory.
    ///
    /// The worker count defaults to the number of available CPU cores (or 1 if this cannot be determined).
    /// The TCP listener is unset; call [`bind`](Self::bind) before running the server.
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
            _preamble: PhantomData,
        }
    }

    /// Converts the server to use a custom preamble type for incoming connections.
    ///
    /// Calling this method will drop any previously configured preamble decode callbacks. Use it before registering preamble handlers if you wish to retain them.
    ///
    /// # Type Parameters
    ///
    /// * `T` – The type to decode as the connection preamble; must implement `bincode::Decode<()>`, `Send`, and `'static`.
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
    /// # let factory = || WireframeApp::new().unwrap();
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
    /// let factory = || WireframeApp::new().unwrap();
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
        H: Fn(&T) + Send + Sync + 'static,
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

    /// Get the configured worker count.
    #[inline]
    #[must_use]
    /// Returns the configured number of worker tasks for the server.
    ///
    /// # Examples
    ///
    /// ```no_run
    /// use wireframe::{app::WireframeApp, server::WireframeServer};
    ///
    /// let factory = || WireframeApp::new().unwrap();
    /// let server = WireframeServer::new(factory);
    /// assert!(server.worker_count() >= 1);
    /// ```
    pub const fn worker_count(&self) -> usize {
        self.workers
    }

    /// Get the socket address the server is bound to, if available.
    #[must_use]
    pub fn local_addr(&self) -> Option<SocketAddr> {
        self.listener.as_ref().and_then(|l| l.local_addr().ok())
    }

    /// Bind the server to the given address and create a listener.
    ///
    /// # Errors
    ///
    /// Binds the server to the specified socket address and prepares it for accepting TCP connections.
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
    /// use wireframe::{app::WireframeApp, server::WireframeServer};
    ///
    /// let factory = || WireframeApp::new().unwrap();
    /// let server = WireframeServer::new(factory);
    /// let addr: SocketAddr = "127.0.0.1:8080".parse().unwrap();
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
    /// Spawns the configured number of worker tasks, each accepting incoming connections using a shared listener and a separate `WireframeApp` instance. The server listens for a Ctrl+C signal to initiate graceful shutdown, signalling all workers to stop accepting new connections. Waits for all worker tasks to complete before returning.
    ///
    /// # Panics
    ///
    /// Panics if called before `bind` has been invoked.
    ///
    /// # Returns
    ///
    /// Returns `Ok(())` when the server shuts down gracefully, or an `io::Error` if accepting connections fails during runtime.
    ///
    /// # Examples
    ///
    /// ```no_run
    /// use std::net::SocketAddr;
    /// use wireframe::{app::WireframeApp, server::WireframeServer};
    /// async fn run_server() -> std::io::Result<()> {
    ///     let factory = || WireframeApp::new().unwrap();
    ///     let server = WireframeServer::new(factory)
    ///         .workers(4)
    ///         .bind("127.0.0.1:8080".parse::<SocketAddr>().unwrap())?;
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
        let (shutdown_tx, _) = broadcast::channel(16);

        // Spawn worker tasks.
        let mut handles = Vec::with_capacity(self.workers);
        for _ in 0..self.workers {
            let listener = Arc::clone(&listener);
            let factory = self.factory.clone();
            let on_success = self.on_preamble_success.clone();
            let on_failure = self.on_preamble_failure.clone();
            let mut shutdown_rx = shutdown_tx.subscribe();
            handles.push(tokio::spawn(async move {
                worker_task(listener, factory, on_success, on_failure, &mut shutdown_rx).await;
            }));
        }

        let join_all = futures::future::join_all(handles);
        tokio::pin!(join_all);

        tokio::select! {
            () = shutdown => {
                let _ = shutdown_tx.send(());
            }
            _ = &mut join_all => {}
        }

        for res in join_all.await {
            if let Err(e) = res {
                eprintln!("worker task failed: {e}");
            }
        }
        Ok(())
    }
}

#[allow(clippy::type_complexity)]
/// Runs a worker task that accepts incoming TCP connections and processes them asynchronously.
///
/// Each accepted connection is handled in a separate task, with optional callbacks for preamble decode success or failure. The worker listens for shutdown signals to terminate gracefully. Accept errors are retried with exponential backoff.
async fn worker_task<F, T>(
    listener: Arc<TcpListener>,
    factory: F,
    on_success: Option<Arc<dyn Fn(&T) + Send + Sync>>,
    on_failure: Option<Arc<dyn Fn(&DecodeError) + Send + Sync>>,
    shutdown_rx: &mut broadcast::Receiver<()>,
) where
    F: Fn() -> WireframeApp + Send + Sync + Clone + 'static,
    // `Preamble` ensures `T` supports borrowed decoding.
    T: Preamble,
{
    let mut delay = Duration::from_millis(10);
    loop {
        tokio::select! {
            res = listener.accept() => match res {
                Ok((stream, _)) => {
                    let success = on_success.clone();
                    let failure = on_failure.clone();
                    let factory = factory.clone();
                    tokio::spawn(process_stream(stream, factory, success, failure));
                    delay = Duration::from_millis(10);
                }
                Err(e) => {
                    eprintln!("accept error: {e}");
                    sleep(delay).await;
                    delay = (delay * 2).min(Duration::from_secs(1));
                }
            },
            _ = shutdown_rx.recv() => break,
        }
    }
}

#[allow(clippy::type_complexity)]
/// Processes an incoming TCP stream by decoding a preamble and dispatching the connection to a `WireframeApp`.
///
/// Attempts to asynchronously decode a preamble of type `T` from the provided stream. If decoding succeeds, invokes the optional success handler, wraps the stream to include any leftover bytes, and passes it to a new `WireframeApp` instance for connection handling. If decoding fails, invokes the optional failure handler and closes the connection.
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
    on_success: Option<Arc<dyn Fn(&T) + Send + Sync>>,
    on_failure: Option<Arc<dyn Fn(&DecodeError) + Send + Sync>>,
) where
    F: Fn() -> WireframeApp + Send + Sync + 'static,
    // `Preamble` ensures `T` supports borrowed decoding.
    T: Preamble,
{
    match read_preamble::<_, T>(&mut stream).await {
        Ok((preamble, leftover)) => {
            if let Some(handler) = on_success.as_ref() {
                handler(&preamble);
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
