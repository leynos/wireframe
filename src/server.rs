use std::io;
use std::net::{SocketAddr, TcpListener as StdTcpListener};
use std::sync::Arc;

use tokio::net::TcpListener;
use tokio::sync::broadcast;
use tokio::time::{Duration, sleep};

use core::marker::PhantomData;

use crate::preamble::read_preamble;
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
    // `Decode`'s type parameter represents a decoding context.
    // The unit type signals that no context is required.
    T: bincode::Decode<()> + Send + 'static,
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
    /// Constructs a new `WireframeServer` using the provided application factory
    /// closure.
    ///
    /// The default worker count equals the number of CPU cores.
    ///
    /// If the CPU count cannot be determined, the server defaults to a single
    /// worker.
    ///
    /// ```ignore
    /// use wireframe::{app::WireframeApp, server::WireframeServer};
    ///
    /// let factory = || WireframeApp::new().unwrap();
    /// let server = WireframeServer::new(factory);
    /// ```
    ///
    /// Creates a new `WireframeServer` with the provided factory closure.
    ///
    /// The server is initialised with a default worker count equal to the number of available CPU cores, or 1 if this cannot be determined. The TCP listener is unset and must be configured with `bind` before running the server.
    ///
    /// # Panics
    ///
    /// Panics if the number of available CPUs cannot be determined and the fallback to 1 fails.
    ///
    /// # Examples
    ///
    /// ```ignore
    /// use wireframe::{app::WireframeApp, server::WireframeServer};
    ///
    /// let server = WireframeServer::new(|| WireframeApp::default());
    /// assert!(server.worker_count() >= 1);
    /// ```
    #[must_use]
    pub fn new(factory: F) -> Self {
        let workers = std::thread::available_parallelism().map_or(1, std::num::NonZeroUsize::get);
        Self {
            factory,
            listener: None,
            workers,
            on_preamble_success: None,
            on_preamble_failure: None,
            _preamble: PhantomData,
        }
    }

    /// Convert this server to parse a custom preamble `T`.
    ///
    /// Call this before registering preamble handlers, otherwise any
    /// previously configured callbacks will be dropped.
    #[must_use]
    pub fn with_preamble<T>(self) -> WireframeServer<F, T>
    where
        // Unit context indicates no external state is required when decoding.
        T: bincode::Decode<()> + Send + 'static,
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
    // `Decode` is generic over a context type; we use `()` here.
    T: bincode::Decode<()> + Send + 'static,
{
    /// Set the number of worker tasks to spawn for the server.
    ///
    /// Ensures at least one worker is configured.
    ///
    /// # Examples
    ///
    /// ```no_run
    /// use wireframe::{app::WireframeApp, server::WireframeServer};
    ///
    /// let factory = || WireframeApp::new().unwrap();
    /// let server = WireframeServer::new(factory).workers(4);
    /// ```
    #[must_use]
    /// Sets the number of worker tasks to spawn, ensuring at least one worker is configured.
    ///
    /// Returns a new `WireframeServer` instance with the updated worker count. If `count` is less than 1, it defaults to 1.
    ///
    /// # Examples
    ///
    /// ```ignore
    /// use wireframe::{app::WireframeApp, server::WireframeServer};
    ///
    /// let factory = || WireframeApp::new().unwrap();
    /// let server = WireframeServer::new(factory).workers(4);
    /// assert_eq!(server.worker_count(), 4);
    /// let server = server.workers(0);
    /// assert_eq!(server.worker_count(), 1);
    /// ```
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
    /// ```ignore
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
            let on_success = self.on_preamble_success.clone();
            let on_failure = self.on_preamble_failure.clone();
            let mut shutdown_rx = shutdown_tx.subscribe();
            handles.push(tokio::spawn(async move {
                worker_task(listener, on_success, on_failure, &mut shutdown_rx).await;
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
async fn worker_task<T>(
    listener: Arc<TcpListener>,
    on_success: Option<Arc<dyn Fn(&T) + Send + Sync>>,
    on_failure: Option<Arc<dyn Fn(&DecodeError) + Send + Sync>>,
    shutdown_rx: &mut broadcast::Receiver<()>,
) where
    // The unit context indicates no additional state is needed to decode `T`.
    T: bincode::Decode<()> + Send + 'static,
{
    let mut delay = Duration::from_millis(10);
    loop {
        tokio::select! {
            res = listener.accept() => match res {
                Ok((stream, _)) => {
                    let success = on_success.clone();
                    let failure = on_failure.clone();
                    tokio::spawn(process_stream(stream, success, failure));
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
async fn process_stream<T>(
    mut stream: tokio::net::TcpStream,
    on_success: Option<Arc<dyn Fn(&T) + Send + Sync>>,
    on_failure: Option<Arc<dyn Fn(&DecodeError) + Send + Sync>>,
) where
    // The decoding context parameter is `()`; no external state is needed.
    T: bincode::Decode<()> + Send + 'static,
{
    match read_preamble::<_, T>(&mut stream).await {
        Ok((preamble, leftover)) => {
            let _ = &leftover; // retain for future replay logic
            if let Some(handler) = on_success.as_ref() {
                handler(&preamble);
            }
            // TODO: wrap `stream` so that `leftover` is replayed before
            // delegating to the underlying socket (e.g. `RewindableStream`).
        }
        Err(err) => {
            if let Some(handler) = on_failure.as_ref() {
                handler(&err);
            }
            // drop stream on failure
        }
    }
}
