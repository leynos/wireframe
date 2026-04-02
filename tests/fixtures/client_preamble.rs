//! `ClientPreambleWorld` fixture for rstest-bdd tests.
//!
//! Provides server/client coordination for preamble exchange scenarios.
#![expect(
    clippy::excessive_nesting,
    reason = "async closures within builder patterns are inherently nested"
)]

use std::{net::SocketAddr, sync::Arc, time::Duration};

use futures::FutureExt;
use rstest::fixture;
use tokio::{io::AsyncWriteExt, net::TcpListener, sync::oneshot, task::JoinHandle};
use wireframe::{
    client::{ClientError, WireframeClient},
    preamble::{read_preamble, write_preamble},
    rewind_stream::RewindStream,
    serializer::BincodeSerializer,
};
/// `TestResult` for step definitions.
pub use wireframe_testing::TestResult;

mod support;

/// Invalid acknowledgement bytes used to exercise preamble-read error-handling.
pub const INVALID_ACK_BYTES: [u8; 3] = [0xff, 0x00, 0x01];

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

fn preamble_decode_error(error: bincode::error::DecodeError) -> std::io::Error {
    std::io::Error::new(std::io::ErrorKind::InvalidData, error)
}

fn preamble_encode_error(error: bincode::error::EncodeError) -> std::io::Error {
    std::io::Error::new(std::io::ErrorKind::InvalidData, error)
}

/// Construct a preamble-failure callback that signals `holder` and returns `Ok`.
fn make_failure_signal_callback(
    holder: SenderHolder<()>,
) -> impl for<'a> Fn(
    &'a ClientError,
    &'a mut tokio::net::TcpStream,
) -> futures::future::BoxFuture<'a, std::io::Result<()>> {
    move |_err, _stream| {
        let holder = holder.clone();
        async move {
            send_signal(&holder, ());
            Ok(())
        }
        .boxed()
    }
}

/// Test world exercising client preamble exchange.
#[derive(Debug, Default)]
pub struct ClientPreambleWorld {
    addr: Option<SocketAddr>,
    server: Option<JoinHandle<std::io::Result<()>>>,
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
    async fn spawn_server<F, Fut>(&mut self, handler: F) -> TestResult
    where
        F: FnOnce(tokio::net::TcpStream) -> Fut + Send + 'static,
        Fut: std::future::Future<Output = std::io::Result<()>> + Send,
    {
        let listener = TcpListener::bind("127.0.0.1:0").await?;
        let addr = listener.local_addr()?;
        let handle = tokio::spawn(async move {
            let (stream, _) = listener.accept().await?;
            handler(stream).await
        });
        self.addr = Some(addr);
        self.server = Some(handle);
        Ok(())
    }

    /// Bind a listener, accept the first connection, read and discard the client
    /// preamble, then call `handler` with the bare stream.
    async fn spawn_server_after_preamble<F, Fut>(&mut self, handler: F) -> TestResult
    where
        F: FnOnce(tokio::net::TcpStream) -> Fut + Send + 'static,
        Fut: std::future::Future<Output = std::io::Result<()>> + Send,
    {
        self.spawn_server(|mut stream| async move {
            read_preamble::<_, TestPreamble>(&mut stream)
                .await
                .map_err(preamble_decode_error)?;
            handler(stream).await
        })
        .await
    }

    /// Store the outcome of a connect attempt that carries no failure signal.
    ///
    /// On success the client is retained; on failure the error is recorded.
    fn store_connect_result(
        &mut self,
        result: Result<
            WireframeClient<BincodeSerializer, RewindStream<tokio::net::TcpStream>>,
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

    /// Store the outcome of a connect attempt that exposes a failure-callback signal.
    ///
    /// On success the client is retained. On failure the error is recorded and,
    /// if the failure signal fires within one second, `failure_callback_invoked`
    /// is set.
    async fn store_connect_result_with_failure_signal(
        &mut self,
        result: Result<
            WireframeClient<BincodeSerializer, RewindStream<tokio::net::TcpStream>>,
            ClientError,
        >,
        failure_rx: oneshot::Receiver<()>,
    ) {
        match result {
            Ok(client) => {
                self.client = Some(client);
            }
            Err(e) => {
                self.last_error = Some(e);
                if matches!(
                    tokio::time::timeout(Duration::from_secs(1), failure_rx).await,
                    Ok(Ok(()))
                ) {
                    self.failure_callback_invoked = true;
                }
            }
        }
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

    /// Check if last error was a preamble read failure.
    #[must_use]
    pub fn was_preamble_read_error(&self) -> bool {
        matches!(self.last_error, Some(ClientError::PreambleRead(_)))
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
