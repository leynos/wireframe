use std::io;
use std::net::{SocketAddr, TcpListener as StdTcpListener};
use std::sync::Arc;

use tokio::net::TcpListener;
use tokio::sync::broadcast;
use tokio::time::{Duration, sleep};

use crate::app::WireframeApp;

/// Tokio-based server for `WireframeApp` instances.
///
/// `WireframeServer` spawns a worker task per thread. Each worker
/// receives its own `WireframeApp` from the provided factory
/// closure. The server listens for a shutdown signal using
/// `tokio::signal::ctrl_c` and notifies all workers to stop
/// accepting new connections.
pub struct WireframeServer<F>
where
    F: Fn() -> WireframeApp + Send + Sync + Clone + 'static,
{
    factory: F,
    listener: Option<Arc<TcpListener>>,
    workers: usize,
}

impl<F> WireframeServer<F>
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
        }
    }

    /// Set the number of worker tasks to spawn for the server.
    ///
    /// Ensures at least one worker is configured.
    ///
    /// # Examples
    ///
    /// ```ignore
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
    /// let server = WireframeServer::new(factory).workers(4);
    /// assert_eq!(server.worker_count(), 4);
    /// let server = server.workers(0);
    /// assert_eq!(server.worker_count(), 1);
    /// ```
    pub fn workers(mut self, count: usize) -> Self {
        self.workers = count.max(1);
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
    /// let server = WireframeServer::new(factory);
    /// assert!(server.worker_count() >= 1);
    /// ```
    pub const fn worker_count(&self) -> usize {
        self.workers
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
    /// ```ignore
    /// use std::net::SocketAddr;
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
    /// ```ignore
    /// # use std::net::SocketAddr;
    /// # use mycrate::{WireframeServer, WireframeApp};
    /// # async fn run_server() -> std::io::Result<()> {
    /// let factory = || WireframeApp::new();
    /// let server = WireframeServer::new(factory)
    ///     .workers(4)
    ///     .bind("127.0.0.1:8080".parse::<SocketAddr>().unwrap())?;
    /// server.run().await
    /// # }
    /// ```
    pub async fn run(self) -> io::Result<()> {
        let listener = self.listener.expect("`bind` must be called before `run`");
        let (shutdown_tx, _) = broadcast::channel(16);

        // Spawn worker tasks using Tokio's runtime.
        let mut handles = Vec::with_capacity(self.workers);
        for _ in 0..self.workers {
            let mut shutdown_rx = shutdown_tx.subscribe();
            let listener = Arc::clone(&listener);
            let factory = self.factory.clone();
            handles.push(tokio::spawn(async move {
                let app = (factory)();
                let mut delay = Duration::from_millis(10);
                loop {
                    tokio::select! {
                        res = listener.accept() => match res {
                            Ok((_stream, _)) => {
                                // TODO: hand off stream to `app`
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
                drop(app);
            }));
        }

        // Wait for Ctrl+C or workers finishing.
        let join_all = futures::future::join_all(handles);
        tokio::pin!(join_all);

        tokio::select! {
            _ = tokio::signal::ctrl_c() => {
                let _ = shutdown_tx.send(());
            }
            _ = &mut join_all => {}
        }

        // Ensure all workers have exited before returning.
        join_all.await;
        Ok(())
    }
}
