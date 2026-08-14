//! `ClientLifecycleWorld` fixture for rstest-bdd tests.
//!
//! Provides server/client coordination for lifecycle hook scenarios.

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
use rstest::fixture;
use tokio::{net::TcpListener, task::JoinHandle};
use wireframe::{
    client::{ClientError, WireframeClient},
    preamble::{read_preamble, write_preamble},
    rewind_stream::RewindStream,
    serializer::BincodeSerializer,
};
/// Re-export `TestResult` from `wireframe_testing` for use in steps.
pub use wireframe_testing::TestResult;

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

/// State value returned by the setup callback and expected by teardown tests.
///
/// This constant defines the sentinel value used to verify that teardown hooks
/// receive the correct state from setup hooks.
pub const EXPECTED_SETUP_STATE: u32 = 42;

/// Client type alias for lifecycle tests.
///
/// Uses `BincodeSerializer` with a `RewindStream` over TCP and `u32` connection state.
type TestClient = WireframeClient<BincodeSerializer, RewindStream<tokio::net::TcpStream>, u32>;

/// Test world exercising client lifecycle hooks.
///
/// The world owns the runtime for the whole scenario so the server task, the
/// client that connects to it, and the final join all run on one runtime. A
/// per-step runtime would be dropped when its step returned, taking the server
/// task with it.
#[derive(Debug)]
pub struct ClientLifecycleWorld {
    runtime: tokio::runtime::Runtime,
    addr: Option<SocketAddr>,
    server: Option<JoinHandle<TestResult>>,
    client: Option<TestClient>,
    setup_count: Arc<AtomicUsize>,
    teardown_count: Arc<AtomicUsize>,
    teardown_received_state: Arc<AtomicUsize>,
    error_count: Arc<AtomicUsize>,
    preamble_success_invoked: Arc<AtomicBool>,
    last_error: Option<ClientError>,
}

impl Drop for ClientLifecycleWorld {
    /// Aborts any server task a scenario did not explicitly finish.
    ///
    /// This is a fallback: [`ClientLifecycleWorld::finish_server`] is the path
    /// that actually observes the task's result.
    fn drop(&mut self) {
        if let Some(handle) = self.server.take() {
            handle.abort();
        }
    }
}

/// Fixture for `ClientLifecycleWorld`.
///
/// Building the runtime can fail, so the fixture reports that to the scenario
/// rather than panicking during arrangement.
#[rustfmt::skip]
#[fixture]
pub fn client_lifecycle_world() -> TestResult<ClientLifecycleWorld> {
    Ok(ClientLifecycleWorld {
        runtime: tokio::runtime::Runtime::new()?,
        addr: None,
        server: None,
        client: None,
        setup_count: Arc::new(AtomicUsize::new(0)),
        teardown_count: Arc::new(AtomicUsize::new(0)),
        teardown_received_state: Arc::new(AtomicUsize::new(0)),
        error_count: Arc::new(AtomicUsize::new(0)),
        preamble_success_invoked: Arc::new(AtomicBool::new(false)),
        last_error: None,
    })
}

impl ClientLifecycleWorld {
    async fn spawn_server<F, Fut>(&mut self, behaviour: F) -> TestResult
    where
        F: FnOnce(tokio::net::TcpStream) -> Fut + Send + 'static,
        Fut: std::future::Future<Output = TestResult> + Send,
    {
        let listener = TcpListener::bind("127.0.0.1:0").await?;
        let addr = listener.local_addr()?;
        let handle = tokio::spawn(async move {
            let (stream, _) = listener.accept().await?;
            behaviour(stream).await
        });

        self.addr = Some(addr);
        self.server = Some(handle);
        Ok(())
    }

    fn handle_connection_result(&mut self, result: Result<TestClient, ClientError>) {
        match result {
            Ok(client) => {
                self.client = Some(client);
            }
            Err(e) => {
                self.last_error = Some(e);
            }
        }
    }

    async fn connect_with_builder<F, P>(&mut self, configure: F) -> TestResult
    where
        F: FnOnce(
            wireframe::client::WireframeClientBuilder,
        ) -> wireframe::client::WireframeClientBuilder<BincodeSerializer, P, u32>,
        P: bincode::Encode + Send + Sync + 'static,
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
    pub async fn start_standard_server(&mut self) -> TestResult {
        self.spawn_server(|_stream| async {
            tokio::time::sleep(Duration::from_millis(100)).await;
            Ok(())
        })
        .await
    }

    /// Start a server that disconnects immediately after accepting.
    ///
    /// # Errors
    /// Returns an error if binding fails.
    pub async fn start_disconnecting_server(&mut self) -> TestResult {
        self.spawn_server(|stream| async {
            drop(stream);
            Ok(())
        })
        .await
    }

    /// Start a preamble-aware server that sends acknowledgement.
    ///
    /// # Errors
    /// Returns an error if binding fails.
    pub async fn start_ack_server(&mut self) -> TestResult {
        self.spawn_server(|mut stream| async move {
            let (_preamble, _) = read_preamble::<_, TestPreamble>(&mut stream).await?;
            write_preamble(&mut stream, &ServerAck { accepted: true }).await?;
            tokio::time::sleep(Duration::from_millis(100)).await;
            Ok(())
        })
        .await
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
                    EXPECTED_SETUP_STATE
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
        let setup_count = Arc::clone(&self.setup_count);
        let teardown_count = Arc::clone(&self.teardown_count);
        let teardown_received_state = Arc::clone(&self.teardown_received_state);

        self.connect_with_builder(|builder| {
            builder
                .on_connection_setup(move || {
                    let count = setup_count.clone();
                    async move {
                        count.fetch_add(1, Ordering::SeqCst);
                        EXPECTED_SETUP_STATE
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
        })
        .await
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
        let setup_count = Arc::clone(&self.setup_count);
        let preamble_invoked = Arc::clone(&self.preamble_success_invoked);

        self.connect_with_builder(|builder| {
            builder
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
                        EXPECTED_SETUP_STATE
                    }
                })
        })
        .await
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

    /// Handle to the scenario's runtime.
    ///
    /// Returning an owned handle ends the borrow of `self`, so a step can drive
    /// a `&mut self` async method on the world's own runtime.
    #[must_use]
    pub fn handle(&self) -> tokio::runtime::Handle { self.runtime.handle().clone() }

    /// Get a reference to the last captured error, if any.
    #[must_use]
    pub fn last_error(&self) -> Option<&ClientError> { self.last_error.as_ref() }

    /// Await the server task and surface both join and inner errors.
    ///
    /// Without this the task's `TestResult` is only ever aborted on drop, so a
    /// server-side failure would pass unnoticed.
    ///
    /// # Errors
    /// Returns an error if no server was started, if the task panicked or was
    /// cancelled, or if the server itself failed.
    pub async fn finish_server(&mut self) -> TestResult {
        let handle = self.server.take().ok_or("server task missing")?;
        handle.await??;
        Ok(())
    }
}
