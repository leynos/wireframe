use std::io;
use std::net::SocketAddr;
use std::sync::Arc;

use tokio::net::TcpListener;
use tokio::sync::broadcast;

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
    addr: Option<SocketAddr>,
    workers: usize,
}

impl<F> WireframeServer<F>
where
    F: Fn() -> WireframeApp + Send + Sync + Clone + 'static,
{
    /// Create a new server from an application factory.
    pub fn new(factory: F) -> Self {
        Self {
            factory,
            addr: None,
            workers: num_cpus::get(),
        }
    }

    /// Set the number of worker tasks to spawn.
    #[must_use]
    pub fn workers(mut self, count: usize) -> Self {
        self.workers = count.max(1);
        self
    }

    /// Bind the server to the given address.
    ///
    /// # Errors
    ///
    /// Returns an [`io::Error`] if binding fails.
    pub fn bind(mut self, addr: SocketAddr) -> io::Result<Self> {
        self.addr = Some(addr);
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
    /// Panics if called before [`bind`].
    pub async fn run(self) -> io::Result<()> {
        let addr = self.addr.expect("`bind` must be called before `run`");
        let listener = TcpListener::bind(addr).await?;
        let listener = Arc::new(listener);
        let (shutdown_tx, _) = broadcast::channel(1);

        // Spawn worker tasks using Tokio's runtime.
        let mut handles = Vec::with_capacity(self.workers);
        for _ in 0..self.workers {
            let mut shutdown_rx = shutdown_tx.subscribe();
            let listener = Arc::clone(&listener);
            let factory = self.factory.clone();
            handles.push(tokio::spawn(async move {
                let _app = (factory)();
                loop {
                    tokio::select! {
                        res = listener.accept() => {
                            match res {
                                Ok((_stream, _)) => {
                                    // TODO: pass stream to application
                                }
                                Err(e) => eprintln!("accept error: {e}"),
                            }
                        }
                        _ = shutdown_rx.recv() => break,
                    }
                }
            }));
        }

        // Wait for Ctrl+C for graceful shutdown.
        tokio::select! {
            _ = tokio::signal::ctrl_c() => {
                let _ = shutdown_tx.send(());
            }
            _ = futures::future::join_all(handles) => {}
        }
        Ok(())
    }
}
