//! Test world state for Cucumber panic resilience tests.
//!
//! Provides shared state management for behavioural tests verifying
//! server resilience against connection task panics.

use std::net::SocketAddr;

use cucumber::World;
use tokio::{net::TcpStream, sync::oneshot};
use wireframe::{
    app::{Envelope, WireframeApp},
    server::WireframeServer,
};

#[derive(Debug)]
struct PanicServer {
    addr: SocketAddr,
    shutdown: Option<oneshot::Sender<()>>,
    handle: tokio::task::JoinHandle<()>,
}

impl PanicServer {
    async fn spawn() -> Self {
        let factory = || {
            WireframeApp::<_, _, Envelope>::new()
                .expect("Failed to create WireframeApp")
                .on_connection_setup(|| async { panic!("boom") })
                .expect("Failed to set connection setup callback")
        };
        let server = WireframeServer::new(factory)
            .workers(1)
            .bind("127.0.0.1:0".parse().expect("Failed to parse address"))
            .expect("bind");

        let addr = server.local_addr().expect("Failed to get server address");
        let (tx_shutdown, rx_shutdown) = oneshot::channel();
        let (tx_ready, rx_ready) = oneshot::channel();

        let handle = tokio::spawn(async move {
            server
                .ready_signal(tx_ready)
                .run_with_shutdown(async {
                    let _ = rx_shutdown.await;
                })
                .await
                .expect("Server task failed");
        });
        rx_ready.await.expect("Server did not signal ready");

        Self {
            addr,
            shutdown: Some(tx_shutdown),
            handle,
        }
    }
}

impl Drop for PanicServer {
    fn drop(&mut self) {
        use std::time::Duration;

        if let Some(tx) = self.shutdown.take() {
            let _ = tx.send(());
        }
        let timeout = Duration::from_secs(5);
        let joined = futures::executor::block_on(tokio::time::timeout(timeout, &mut self.handle));
        match joined {
            Ok(Ok(())) => {}
            Ok(Err(e)) => eprintln!("PanicServer task panicked: {e:?}"),
            Err(_) => eprintln!("PanicServer task did not shut down within timeout"),
        }
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
    /// # Panics
    /// Panics if binding the server fails or the server task fails.
    pub async fn start_panic_server(&mut self) { self.server.replace(PanicServer::spawn().await); }

    /// Connect to the running server once.
    ///
    /// # Panics
    /// Panics if the server address is unknown or the connection fails.
    pub async fn connect_once(&mut self) {
        let addr = self.server.as_ref().expect("Server not started").addr;
        TcpStream::connect(addr).await.expect("Failed to connect");
        self.attempts += 1;
    }

    /// Verify both connections succeeded and shut down the server.
    ///
    /// # Panics
    /// Panics if the connection attempts do not match the expected count.
    pub async fn verify_and_shutdown(&mut self) {
        assert_eq!(self.attempts, 2);
        // dropping PanicServer will shut it down
        self.server.take();
        tokio::task::yield_now().await;
    }
}
