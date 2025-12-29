//! Test world for client lifecycle hook scenarios.
#![cfg(not(loom))]
#![expect(
    clippy::expect_used,
    reason = "test code uses expect for concise assertions"
)]
#![expect(
    clippy::excessive_nesting,
    reason = "async closures within builder patterns are inherently nested"
)]

use std::{
    net::SocketAddr,
    sync::{
        Arc,
        atomic::{AtomicBool, AtomicUsize, Ordering},
    },
    time::Duration,
};

use futures::FutureExt;
use tokio::{net::TcpListener, task::JoinHandle};
use wireframe::{
    BincodeSerializer,
    client::{ClientError, WireframeClient},
    preamble::{read_preamble, write_preamble},
    rewind_stream::RewindStream,
};

use super::TestResult;

/// Preamble used for testing lifecycle with preamble.
#[derive(Debug, Clone, PartialEq, Eq, Default, bincode::Encode, bincode::BorrowDecode)]
pub struct TestPreamble {
    version: u16,
}

impl TestPreamble {
    /// Create a new test preamble with the given version.
    #[must_use]
    pub fn new(version: u16) -> Self { Self { version } }
}

/// Server acknowledgement preamble.
#[derive(Debug, Clone, PartialEq, Eq, Default, bincode::Encode, bincode::BorrowDecode)]
pub struct ServerAck {
    accepted: bool,
}

/// Test world exercising client lifecycle hooks.
#[derive(Debug, Default, cucumber::World)]
pub struct ClientLifecycleWorld {
    addr: Option<SocketAddr>,
    server: Option<JoinHandle<()>>,
    client: Option<WireframeClient<BincodeSerializer, RewindStream<tokio::net::TcpStream>, u32>>,
    setup_count: Arc<AtomicUsize>,
    teardown_count: Arc<AtomicUsize>,
    teardown_received_state: Arc<AtomicUsize>,
    error_count: Arc<AtomicUsize>,
    preamble_success_invoked: Arc<AtomicBool>,
    last_error: Option<ClientError>,
}

impl ClientLifecycleWorld {
    /// Spawn a server that executes the provided behaviour closure after accepting a connection.
    ///
    /// This helper binds a `TcpListener`, stores the address in `self.addr`, spawns a task
    /// that accepts a connection and runs the closure, and stores the task handle in
    /// `self.server`.
    async fn spawn_server<F, Fut>(&mut self, behaviour: F) -> TestResult
    where
        F: FnOnce(tokio::net::TcpStream) -> Fut + Send + 'static,
        Fut: std::future::Future<Output = ()> + Send,
    {
        let listener = TcpListener::bind("127.0.0.1:0").await?;
        let addr = listener.local_addr()?;
        let handle = tokio::spawn(async move {
            let (stream, _) = listener.accept().await.expect("accept");
            behaviour(stream).await;
        });

        self.addr = Some(addr);
        self.server = Some(handle);
        Ok(())
    }

    /// Handle the result of a client connection attempt.
    ///
    /// Stores the client in `self.client` on success, or the error in `self.last_error`
    /// on failure.
    fn handle_connection_result(
        &mut self,
        result: Result<
            WireframeClient<BincodeSerializer, RewindStream<tokio::net::TcpStream>, u32>,
            ClientError,
        >,
    ) {
        match result {
            Ok(client) => {
                self.client = Some(client);
            }
            Err(e) => {
                self.last_error = Some(e);
            }
        }
    }

    /// Connect using a builder configuration closure.
    ///
    /// This helper retrieves the server address, applies the provided configuration
    /// to a new builder, connects, and handles the result.
    async fn connect_with_builder<F>(&mut self, configure: F) -> TestResult
    where
        F: FnOnce(
            wireframe::client::WireframeClientBuilder,
        ) -> wireframe::client::WireframeClientBuilder<BincodeSerializer, (), u32>,
    {
        let addr = self.addr.ok_or("server address missing")?;
        let result = configure(WireframeClient::builder()).connect(addr).await;
        self.handle_connection_result(result);
        Ok(())
    }

    /// Start a standard echo server.
    ///
    /// # Errors
    /// Returns an error if binding fails.
    ///
    /// # Panics
    /// The spawned task panics if accept fails.
    pub async fn start_standard_server(&mut self) -> TestResult {
        self.spawn_server(|_stream| async {
            // Hold connection briefly
            tokio::time::sleep(Duration::from_millis(100)).await;
        })
        .await
    }

    /// Start a server that disconnects immediately after accepting.
    ///
    /// # Errors
    /// Returns an error if binding fails.
    ///
    /// # Panics
    /// The spawned task panics if accept fails.
    pub async fn start_disconnecting_server(&mut self) -> TestResult {
        self.spawn_server(|stream| async {
            // Disconnect immediately
            drop(stream);
        })
        .await
    }

    /// Start a preamble-aware server that sends acknowledgement.
    ///
    /// # Errors
    /// Returns an error if binding fails.
    ///
    /// # Panics
    /// The spawned task panics if accept, preamble read, or ack write fails.
    pub async fn start_ack_server(&mut self) -> TestResult {
        let listener = TcpListener::bind("127.0.0.1:0").await?;
        let addr = listener.local_addr()?;
        let handle = tokio::spawn(async move {
            let (mut stream, _) = listener.accept().await.expect("accept");
            let (_preamble, _) = read_preamble::<_, TestPreamble>(&mut stream)
                .await
                .expect("read preamble");
            write_preamble(&mut stream, &ServerAck { accepted: true })
                .await
                .expect("write ack");
            tokio::time::sleep(Duration::from_millis(100)).await;
        });

        self.addr = Some(addr);
        self.server = Some(handle);
        Ok(())
    }

    /// Connect with a setup callback.
    ///
    /// # Errors
    /// Returns an error if server address is missing.
    pub async fn connect_with_setup(&mut self) -> TestResult {
        let setup_count = Arc::clone(&self.setup_count);

        self.connect_with_builder(|builder| {
            builder.on_connection_setup(move || {
                let count = setup_count.clone();
                async move {
                    count.fetch_add(1, Ordering::SeqCst);
                    42u32
                }
            })
        })
        .await
    }

    /// Connect with setup and teardown callbacks.
    ///
    /// # Errors
    /// Returns an error if server address is missing.
    pub async fn connect_with_setup_and_teardown(&mut self) -> TestResult {
        let addr = self.addr.ok_or("server address missing")?;
        let setup_count = Arc::clone(&self.setup_count);
        let teardown_count = Arc::clone(&self.teardown_count);
        let teardown_received_state = Arc::clone(&self.teardown_received_state);

        let result = WireframeClient::builder()
            .on_connection_setup(move || {
                let count = setup_count.clone();
                async move {
                    count.fetch_add(1, Ordering::SeqCst);
                    42u32
                }
            })
            .on_connection_teardown(move |state: u32| {
                let count = teardown_count.clone();
                let received = teardown_received_state.clone();
                async move {
                    count.fetch_add(1, Ordering::SeqCst);
                    received.store(state as usize, Ordering::SeqCst);
                }
            })
            .connect(addr)
            .await;

        self.handle_connection_result(result);
        Ok(())
    }

    /// Connect with an error callback.
    ///
    /// # Errors
    /// Returns an error if server address is missing.
    pub async fn connect_with_error_callback(&mut self) -> TestResult {
        let error_count = Arc::clone(&self.error_count);

        self.connect_with_builder(|builder| {
            builder
                .on_connection_setup(|| async { 0u32 })
                .on_error(move |_err| {
                    let count = error_count.clone();
                    async move {
                        count.fetch_add(1, Ordering::SeqCst);
                    }
                })
        })
        .await
    }

    /// Connect with preamble and lifecycle callbacks.
    ///
    /// # Errors
    /// Returns an error if server address is missing.
    ///
    /// # Panics
    /// Asserts if server does not accept preamble.
    pub async fn connect_with_preamble_and_lifecycle(&mut self) -> TestResult {
        let addr = self.addr.ok_or("server address missing")?;
        let setup_count = Arc::clone(&self.setup_count);
        let preamble_invoked = Arc::clone(&self.preamble_success_invoked);

        let result = WireframeClient::builder()
            .with_preamble(TestPreamble::new(1))
            .on_preamble_success(move |_preamble, stream| {
                let invoked = preamble_invoked.clone();
                async move {
                    invoked.store(true, Ordering::SeqCst);
                    let (ack, leftover) =
                        read_preamble::<_, ServerAck>(stream).await.map_err(|e| {
                            std::io::Error::new(std::io::ErrorKind::InvalidData, e.to_string())
                        })?;
                    assert!(ack.accepted, "server should accept preamble");
                    Ok(leftover)
                }
                .boxed()
            })
            .on_connection_setup(move || {
                let count = setup_count.clone();
                async move {
                    count.fetch_add(1, Ordering::SeqCst);
                    42u32
                }
            })
            .connect(addr)
            .await;

        self.handle_connection_result(result);
        Ok(())
    }

    /// Close the client connection.
    pub async fn close_client(&mut self) {
        if let Some(client) = self.client.take() {
            client.close().await;
        }
    }

    /// Attempt to receive a message (should fail after server disconnect).
    ///
    /// # Errors
    /// Returns `Ok` but stores any receive error in `last_error`.
    pub async fn attempt_receive(&mut self) -> TestResult {
        if let Some(ref mut client) = self.client {
            // Wait a bit for server to disconnect
            tokio::time::sleep(Duration::from_millis(50)).await;
            let result: Result<Vec<u8>, ClientError> = client.receive().await;
            if let Err(e) = result {
                self.last_error = Some(e);
            }
        }
        Ok(())
    }

    /// Get the setup callback invocation count.
    #[must_use]
    pub fn setup_count(&self) -> usize { self.setup_count.load(Ordering::SeqCst) }

    /// Get the teardown callback invocation count.
    #[must_use]
    pub fn teardown_count(&self) -> usize { self.teardown_count.load(Ordering::SeqCst) }

    /// Get the state received by teardown callback.
    #[must_use]
    pub fn teardown_received_state(&self) -> usize {
        self.teardown_received_state.load(Ordering::SeqCst)
    }

    /// Get the error callback invocation count.
    #[must_use]
    pub fn error_count(&self) -> usize { self.error_count.load(Ordering::SeqCst) }

    /// Check if preamble success callback was invoked.
    #[must_use]
    pub fn preamble_success_invoked(&self) -> bool {
        self.preamble_success_invoked.load(Ordering::SeqCst)
    }

    /// Abort the server task.
    pub fn abort_server(&mut self) {
        if let Some(handle) = self.server.take() {
            handle.abort();
        }
    }
}
