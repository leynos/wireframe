use std::io;
use std::net::{SocketAddr, TcpListener as StdTcpListener};
use std::sync::Arc;

use tokio::net::TcpListener;
use tokio::sync::broadcast;
use tokio::time::{Duration, sleep};

use crate::app::WireframeApp;

/// Tokio-based server for `WireframeApp` instances.
///
/// Each worker receives its own application instance from the provided
/// factory. A Ctrl+C signal triggers graceful shutdown across all
/// workers.
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
    /// Create a new server using the given application factory.
    #[must_use]
    /// Creates a new server with the specified factory function.
    ///
    /// The server is initialised with no bound listener and a default worker count equal to the number of CPU cores (at least one).
    ///
    /// # Examples
    ///
    /// ```
    /// let server = WireframeServer::new(my_factory);
    /// ```
    pub fn new(factory: F) -> Self {
        Self {
            factory,
            listener: None,
            workers: num_cpus::get().max(1),
        }
    }

    /// Set the number of worker tasks to spawn for the server.
    #[must_use]
    /// Sets the number of worker tasks to spawn, ensuring at least one.
    ///
    /// Returns a new server instance with the updated worker count.
    ///
    /// # Parameters
    ///
    /// - `count`: The desired number of worker tasks. If less than 1, defaults to 1.
    ///
    /// # Examples
    ///
    /// ```
    /// let server = WireframeServer::new(factory).workers(4);
    /// ```
    pub fn workers(mut self, count: usize) -> Self {
        self.workers = count.max(1);
        self
    }

    /// Bind the server to the provided socket address.
    ///
    /// # Errors
    ///
    /// Binds the server to the specified socket address using a non-blocking TCP listener.
    ///
    /// Returns an error if binding or configuring the listener fails.
    pub fn bind(mut self, addr: SocketAddr) -> io::Result<Self> {
        let std_listener = StdTcpListener::bind(addr)?;
        std_listener.set_nonblocking(true)?;
        let listener = TcpListener::from_std(std_listener)?;
        self.listener = Some(Arc::new(listener));
        Ok(self)
    }

    /// Run the server until a shutdown signal is received.
    ///
    /// # Errors
    ///
    /// Returns any I/O error encountered while accepting connections.
    ///
    /// # Panics
    ///
    /// Runs the server, accepting connections and handling shutdown signals.
    ///
    /// Panics if called before `bind` has configured the listener. Spawns worker tasks to accept incoming TCP connections and gracefully shuts down on Ctrl+C.
    ///
    /// # Returns
    ///
    /// An I/O result indicating success or failure.
    ///
    /// # Examples
    ///
    /// ```no_run
    /// # use std::net::SocketAddr;
    /// # use your_crate::{WireframeServer, WireframeApp};
    /// # async fn run_server() -> std::io::Result<()> {
    /// let server = WireframeServer::new(|| WireframeApp::new())
    ///     .bind("127.0.0.1:8080".parse::<SocketAddr>().unwrap())?;
    /// server.run().await
    /// # }
    /// ```
    pub async fn run(self) -> io::Result<()> {
        let listener = self.listener.expect("`bind` must be called before `run`");
        let (shutdown_tx, _) = broadcast::channel(16);

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

        let join_all = futures::future::join_all(handles);
        tokio::pin!(join_all);

        tokio::select! {
            _ = tokio::signal::ctrl_c() => {
                let _ = shutdown_tx.send(());
            }
            _ = &mut join_all => {}
        }

        join_all.await;
        Ok(())
    }
}
