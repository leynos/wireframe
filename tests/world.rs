//! Test world state for Cucumber panic resilience tests.
//!
//! Provides shared state management for behavioural tests verifying
//! server resilience against connection task panics.

use std::net::SocketAddr;

use cucumber::World;
use tokio::{
    net::TcpStream,
    sync::oneshot::{self, Sender},
};
use wireframe::{app::WireframeApp, server::WireframeServer};

#[derive(Debug, Default, World)]
pub struct PanicWorld {
    pub addr: Option<SocketAddr>,
    pub attempts: usize,
    pub shutdown: Option<Sender<()>>,
}

impl PanicWorld {
    /// Start a server that panics during connection setup.
    ///
    /// # Panics
    /// Panics if binding the server fails or the server task fails.
    pub async fn start_panic_server(&mut self) {
        let factory = || {
            WireframeApp::new()
                .expect("Failed to create WireframeApp")
                .on_connection_setup(|| async { panic!("boom") })
                .unwrap()
        };
        let server = WireframeServer::new(factory)
            .workers(1)
            .bind("127.0.0.1:0".parse().expect("Failed to parse address"))
            .expect("bind");

        self.addr = Some(server.local_addr().expect("Failed to get server address"));
        let (tx, rx) = oneshot::channel();
        self.shutdown = Some(tx);

        tokio::spawn(async move {
            server
                .run_with_shutdown(async {
                    let _ = rx.await;
                })
                .await
                .unwrap();
        });

        tokio::task::yield_now().await;
    }

    /// Connect to the running server once.
    ///
    /// # Panics
    /// Panics if the server address is unknown or the connection fails.
    pub async fn connect_once(&mut self) {
        TcpStream::connect(self.addr.expect("Server address not set"))
            .await
            .expect("Failed to connect");
        self.attempts += 1;
    }

    /// Verify both connections succeeded and shut down the server.
    ///
    /// # Panics
    /// Panics if the connection attempts do not match the expected count.
    pub async fn verify_and_shutdown(&mut self) {
        assert_eq!(self.attempts, 2);
        if let Some(tx) = self.shutdown.take() {
            let _ = tx.send(());
        }
        tokio::task::yield_now().await;
    }
}
