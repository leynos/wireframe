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
    /// Create a new server from an application factory.
    pub fn new(factory: F) -> Self {
        Self {
            factory,
            listener: None,
            workers: num_cpus::get(),
        }
    }

    /// Set the number of worker tasks to spawn.
    #[must_use]
    pub fn workers(mut self, count: usize) -> Self {
        self.workers = count.max(1);
        self
    }

    /// Bind the server to the given address and create a listener.
    ///
    /// # Errors
    ///
    /// Returns an [`io::Error`] if binding fails.
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
    /// Panics if called before [`bind`].
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
