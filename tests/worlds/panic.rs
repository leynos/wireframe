//! Test world for panic-on-connection scenarios.
//!
//! Provides [`PanicWorld`] to ensure the server remains resilient when
//! connection setup handlers panic before a client fully connects.
#![cfg(not(loom))]

use std::net::SocketAddr;

use cucumber::World;
use tokio::{net::TcpStream, sync::oneshot};
use wireframe::server::WireframeServer;

use super::{TestApp, unused_listener};

type TestResult<T = ()> = Result<T, Box<dyn std::error::Error + Send + Sync>>;

#[derive(Debug)]
struct PanicServer {
    addr: SocketAddr,
    shutdown: Option<oneshot::Sender<()>>,
    handle: tokio::task::JoinHandle<()>,
}

impl PanicServer {
    async fn spawn() -> TestResult<Self> {
        let factory = || {
            TestApp::new()
                .and_then(|app| app.on_connection_setup(|| async { panic!("boom") }))
                .unwrap_or_else(|err| {
                    tracing::error!("failed to build panic app: {err}");
                    TestApp::default()
                })
        };
        let listener = unused_listener();
        let server = WireframeServer::new(factory)
            .workers(1)
            .bind_existing_listener(listener)?;
        let addr = server.local_addr().ok_or("Failed to get server address")?;
        let (tx_shutdown, rx_shutdown) = oneshot::channel();
        let (tx_ready, rx_ready) = oneshot::channel();

        let handle = tokio::spawn(async move {
            if let Err(err) = server
                .ready_signal(tx_ready)
                .run_with_shutdown(async {
                    let _ = rx_shutdown.await;
                })
                .await
            {
                tracing::error!("server task failed: {err}");
            }
        });
        rx_ready.await.map_err(|_| "Server did not signal ready")?;

        Ok(Self {
            addr,
            shutdown: Some(tx_shutdown),
            handle,
        })
    }
}

impl Drop for PanicServer {
    fn drop(&mut self) {
        use std::{thread, time::Duration};

        if let Some(tx) = self.shutdown.take() {
            let _ = tx.send(());
        }
        let timeout = Duration::from_secs(5);
        let handle = self.handle.abort_handle();
        thread::spawn(move || {
            thread::sleep(timeout);
            handle.abort();
        });
    }
}

#[derive(Debug, Default, World)]
pub struct PanicWorld {
    server: Option<PanicServer>,
    attempts: usize,
}

impl PanicWorld {
    /// Start a server that panics during connection setup.
    ///
    /// # Errors
    /// Returns an error if building the app factory or binding the server
    /// fails.
    pub async fn start_panic_server(&mut self) -> TestResult {
        let server = PanicServer::spawn().await?;
        self.server.replace(server);
        Ok(())
    }

    /// Connect to the running server once.
    ///
    /// # Errors
    /// Returns an error if the server address is unknown or the connection
    /// attempt fails.
    pub async fn connect_once(&mut self) -> TestResult {
        let addr = self.server.as_ref().ok_or("Server not started")?.addr;
        TcpStream::connect(addr)
            .await
            .map_err(|e| std::io::Error::other(format!("Failed to connect: {e}")))?;
        self.attempts += 1;
        Ok(())
    }

    /// Verify both connections succeeded and shut down the server.
    ///
    /// # Errors
    /// Returns an error if the connection attempts do not match the expected
    /// count.
    pub async fn verify_and_shutdown(&mut self) -> TestResult {
        if self.attempts != 2 {
            return Err("expected two successful connection attempts".into());
        }
        // dropping PanicServer will shut it down
        self.server.take();
        tokio::task::yield_now().await;
        Ok(())
    }
}
