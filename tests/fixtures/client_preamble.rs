//! `ClientPreambleWorld` fixture for rstest-bdd tests.
//!
//! Provides server/client coordination for preamble exchange scenarios.

#![expect(
    clippy::expect_used,
    reason = "test code uses expect for concise assertions"
)]
#![expect(
    clippy::excessive_nesting,
    reason = "async closures within builder patterns are inherently nested"
)]

use std::{net::SocketAddr, sync::Arc, time::Duration};

use futures::FutureExt;
use rstest::fixture;
use tokio::{net::TcpListener, sync::oneshot, task::JoinHandle};
use wireframe::{
    BincodeSerializer,
    client::{ClientError, WireframeClient},
    preamble::{read_preamble, write_preamble},
    rewind_stream::RewindStream,
};
/// `TestResult` for step definitions.
pub use wireframe_testing::TestResult;

/// Preamble used for testing.
#[derive(Debug, Clone, PartialEq, Eq, Default, bincode::Encode, bincode::BorrowDecode)]
pub struct TestPreamble {
    magic: [u8; 4],
    version: u16,
}

impl TestPreamble {
    const MAGIC: [u8; 4] = *b"TEST";

    /// Create a new test preamble with the given version.
    #[must_use]
    pub fn new(version: u16) -> Self {
        Self {
            magic: Self::MAGIC,
            version,
        }
    }

    /// Get the version.
    #[must_use]
    pub fn version(&self) -> u16 { self.version }
}

/// Server acknowledgement preamble.
#[derive(Debug, Clone, PartialEq, Eq, Default, bincode::Encode, bincode::BorrowDecode)]
pub struct ServerAck {
    accepted: bool,
}

impl ServerAck {
    /// Check if the connection was accepted.
    #[must_use]
    pub fn accepted(&self) -> bool { self.accepted }
}

type SenderHolder<T> = Arc<std::sync::Mutex<Option<oneshot::Sender<T>>>>;

fn create_signal_channel<T>() -> (SenderHolder<T>, oneshot::Receiver<T>) {
    let (tx, rx) = oneshot::channel();
    (Arc::new(std::sync::Mutex::new(Some(tx))), rx)
}

fn send_signal<T>(holder: &std::sync::Mutex<Option<oneshot::Sender<T>>>, value: T) {
    if let Some(tx) = holder.lock().ok().and_then(|mut guard| guard.take()) {
        let _ = tx.send(value);
    }
}

/// Test world exercising client preamble exchange.
#[derive(Debug, Default)]
pub struct ClientPreambleWorld {
    addr: Option<SocketAddr>,
    server: Option<JoinHandle<()>>,
    client: Option<WireframeClient<BincodeSerializer, RewindStream<tokio::net::TcpStream>>>,
    server_preamble_rx: Option<oneshot::Receiver<TestPreamble>>,
    server_received_preamble: Option<TestPreamble>,
    client_received_ack: Option<ServerAck>,
    success_callback_invoked: bool,
    failure_callback_invoked: bool,
    last_error: Option<ClientError>,
}

/// Fixture for `ClientPreambleWorld`.
#[rustfmt::skip]
#[fixture]
pub fn client_preamble_world() -> ClientPreambleWorld {
    ClientPreambleWorld::default()
}

impl ClientPreambleWorld {
    /// Start a preamble-aware echo server.
    ///
    /// # Errors
    /// Returns an error if binding fails.
    ///
    /// # Panics
    /// The spawned task panics if accept or read fails.
    pub async fn start_preamble_server(&mut self) -> TestResult {
        let listener = TcpListener::bind("127.0.0.1:0").await?;
        let addr = listener.local_addr()?;
        let (tx, rx) = oneshot::channel::<TestPreamble>();
        let handle = tokio::spawn(async move {
            let (mut stream, _) = listener.accept().await.expect("accept");
            let (preamble, _) = read_preamble::<_, TestPreamble>(&mut stream)
                .await
                .expect("read preamble");
            let _ = tx.send(preamble);
            tokio::time::sleep(Duration::from_millis(100)).await;
        });

        self.addr = Some(addr);
        self.server = Some(handle);
        self.server_preamble_rx = Some(rx);
        Ok(())
    }

    /// Start a preamble-aware server that sends acknowledgement.
    ///
    /// # Errors
    /// Returns an error if binding fails.
    ///
    /// # Panics
    /// The spawned task panics if accept, read, or write fails.
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

    /// Start a server that never responds (for timeout testing).
    ///
    /// # Errors
    /// Returns an error if binding fails.
    ///
    /// # Panics
    /// The spawned task panics if accept fails.
    pub async fn start_slow_server(&mut self) -> TestResult {
        let listener = TcpListener::bind("127.0.0.1:0").await?;
        let addr = listener.local_addr()?;
        let handle = tokio::spawn(async move {
            let (_stream, _) = listener.accept().await.expect("accept");
            tokio::time::sleep(Duration::from_secs(10)).await;
        });

        self.addr = Some(addr);
        self.server = Some(handle);
        Ok(())
    }

    /// Start a standard echo server without preamble support.
    ///
    /// # Errors
    /// Returns an error if binding fails.
    ///
    /// # Panics
    /// The spawned task panics if accept fails.
    pub async fn start_standard_server(&mut self) -> TestResult {
        let listener = TcpListener::bind("127.0.0.1:0").await?;
        let addr = listener.local_addr()?;
        let handle = tokio::spawn(async move {
            let (_stream, _) = listener.accept().await.expect("accept");
            tokio::time::sleep(Duration::from_millis(100)).await;
        });

        self.addr = Some(addr);
        self.server = Some(handle);
        Ok(())
    }

    /// Connect with preamble and success callback.
    ///
    /// # Errors
    /// Returns an error if server address is missing.
    pub async fn connect_with_preamble(&mut self, version: u16) -> TestResult {
        let addr = self.addr.ok_or("server address missing")?;
        let (holder, rx) = create_signal_channel::<()>();

        let result = WireframeClient::builder()
            .with_preamble(TestPreamble::new(version))
            .on_preamble_success(move |_preamble, _stream| {
                let holder = holder.clone();
                async move {
                    send_signal(&holder, ());
                    Ok(Vec::new())
                }
                .boxed()
            })
            .connect(addr)
            .await;

        match result {
            Ok(client) => {
                self.client = Some(client);
                if tokio::time::timeout(Duration::from_secs(1), rx)
                    .await
                    .is_ok()
                {
                    self.success_callback_invoked = true;
                }
            }
            Err(e) => {
                self.last_error = Some(e);
            }
        }

        if let Some(preamble_rx) = self.server_preamble_rx.take()
            && let Ok(Ok(preamble)) =
                tokio::time::timeout(Duration::from_secs(1), preamble_rx).await
        {
            self.server_received_preamble = Some(preamble);
        }

        Ok(())
    }

    /// Connect with preamble and read acknowledgement.
    ///
    /// # Errors
    /// Returns an error if server address is missing.
    pub async fn connect_with_ack(&mut self) -> TestResult {
        let addr = self.addr.ok_or("server address missing")?;
        let (holder, rx) = create_signal_channel::<ServerAck>();

        let result = WireframeClient::builder()
            .with_preamble(TestPreamble::new(1))
            .on_preamble_success(move |_preamble, stream| {
                let holder = holder.clone();
                async move {
                    let (ack, leftover) =
                        read_preamble::<_, ServerAck>(stream).await.map_err(|e| {
                            std::io::Error::new(std::io::ErrorKind::InvalidData, e.to_string())
                        })?;
                    send_signal(&holder, ack);
                    Ok(leftover)
                }
                .boxed()
            })
            .connect(addr)
            .await;

        match result {
            Ok(client) => {
                self.client = Some(client);
                if let Ok(Ok(ack)) = tokio::time::timeout(Duration::from_secs(1), rx).await {
                    self.client_received_ack = Some(ack);
                    self.success_callback_invoked = true;
                }
            }
            Err(e) => {
                self.last_error = Some(e);
            }
        }
        Ok(())
    }

    /// Connect with a preamble timeout.
    ///
    /// # Errors
    /// Returns an error if server address is missing.
    pub async fn connect_with_timeout(&mut self, timeout_ms: u64) -> TestResult {
        let addr = self.addr.ok_or("server address missing")?;
        let (failure_holder, failure_rx) = create_signal_channel::<()>();

        let result = WireframeClient::builder()
            .with_preamble(TestPreamble::new(1))
            .preamble_timeout(Duration::from_millis(timeout_ms))
            .on_preamble_success(|_preamble, stream| {
                async move {
                    use tokio::io::AsyncReadExt;
                    let mut buf = [0u8; 1];
                    stream.read_exact(&mut buf).await?;
                    Ok(Vec::new())
                }
                .boxed()
            })
            .on_preamble_failure(move |_err, _stream| {
                let holder = failure_holder.clone();
                async move {
                    send_signal(&holder, ());
                    Ok(())
                }
                .boxed()
            })
            .connect(addr)
            .await;

        match result {
            Ok(client) => {
                self.client = Some(client);
            }
            Err(e) => {
                self.last_error = Some(e);
                if tokio::time::timeout(Duration::from_secs(1), failure_rx)
                    .await
                    .is_ok()
                {
                    self.failure_callback_invoked = true;
                }
            }
        }
        Ok(())
    }

    /// Connect without a preamble.
    ///
    /// # Errors
    /// Returns an error if server address is missing.
    pub async fn connect_without_preamble(&mut self) -> TestResult {
        let addr = self.addr.ok_or("server address missing")?;
        let result = WireframeClient::builder().connect(addr).await;

        match result {
            Ok(client) => {
                self.client = Some(client);
            }
            Err(e) => {
                self.last_error = Some(e);
            }
        }
        Ok(())
    }

    /// Check if the server received the expected preamble version.
    #[must_use]
    pub fn server_received_version(&self) -> Option<u16> {
        self.server_received_preamble
            .as_ref()
            .map(TestPreamble::version)
    }

    /// Check if success callback was invoked.
    #[must_use]
    pub fn success_invoked(&self) -> bool { self.success_callback_invoked }

    /// Check if failure callback was invoked.
    #[must_use]
    pub fn failure_invoked(&self) -> bool { self.failure_callback_invoked }

    /// Check if client received accepted ack.
    #[must_use]
    pub fn ack_accepted(&self) -> bool {
        self.client_received_ack
            .as_ref()
            .is_some_and(ServerAck::accepted)
    }

    /// Check if last error was a timeout.
    #[must_use]
    pub fn was_timeout_error(&self) -> bool {
        matches!(self.last_error, Some(ClientError::PreambleTimeout))
    }

    /// Check if client is connected.
    #[must_use]
    pub fn is_connected(&self) -> bool { self.client.is_some() }

    /// Abort the server task.
    pub fn abort_server(&mut self) {
        if let Some(handle) = self.server.take() {
            handle.abort();
        }
    }
}
