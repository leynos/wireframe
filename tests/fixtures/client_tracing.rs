//! `ClientTracingWorld` fixture for rstest-bdd tracing tests.
//!
//! Provides a thread-local tracing subscriber that captures formatted output
//! into a shared buffer for assertion in step definitions.

#![expect(
    clippy::expect_used,
    reason = "test code uses expect for concise assertions"
)]
#![expect(
    clippy::excessive_nesting,
    reason = "async closures within spawned echo server are inherently nested"
)]

use std::{
    net::SocketAddr,
    sync::{Arc, Mutex},
};

use bytes::Bytes;
use futures::{SinkExt, StreamExt};
use rstest::fixture;
use tokio::{net::TcpListener, task::JoinHandle};
use tokio_util::codec::{Framed, LengthDelimitedCodec};
use wireframe::{
    app::Envelope,
    client::{TracingConfig, WireframeClient},
    rewind_stream::RewindStream,
    serializer::BincodeSerializer,
};
/// Re-export `TestResult` from `wireframe_testing` for use in steps.
pub use wireframe_testing::TestResult;
use wireframe_testing::{ServerMode, process_frame};

/// Client type alias for tracing tests.
type TestClient = WireframeClient<BincodeSerializer, RewindStream<tokio::net::TcpStream>>;

/// Writer that appends formatted tracing output to a shared buffer.
#[derive(Clone, Debug)]
struct CaptureWriter {
    buf: Arc<Mutex<Vec<u8>>>,
}

impl CaptureWriter {
    fn new(buf: Arc<Mutex<Vec<u8>>>) -> Self { Self { buf } }
}

impl std::io::Write for CaptureWriter {
    fn write(&mut self, data: &[u8]) -> std::io::Result<usize> {
        self.buf
            .lock()
            .map_err(|e| std::io::Error::other(e.to_string()))?
            .extend_from_slice(data);
        Ok(data.len())
    }

    fn flush(&mut self) -> std::io::Result<()> { Ok(()) }
}

impl<'a> tracing_subscriber::fmt::MakeWriter<'a> for CaptureWriter {
    type Writer = Self;

    fn make_writer(&'a self) -> Self::Writer { self.clone() }
}

/// Test world for client tracing BDD scenarios.
pub struct ClientTracingWorld {
    addr: Option<SocketAddr>,
    server: Option<JoinHandle<()>>,
    client: Option<TestClient>,
    tracing_config: TracingConfig,
    captured: Arc<Mutex<Vec<u8>>>,
    _subscriber_guard: tracing::subscriber::DefaultGuard,
}

impl Drop for ClientTracingWorld {
    fn drop(&mut self) {
        if let Some(handle) = self.server.take() {
            handle.abort();
        }
    }
}

/// Create a new `ClientTracingWorld` with default tracing config.
fn new_world(config: TracingConfig) -> ClientTracingWorld {
    let captured = Arc::new(Mutex::new(Vec::new()));
    let writer = CaptureWriter::new(Arc::clone(&captured));

    let subscriber = tracing_subscriber::fmt()
        .with_writer(writer)
        .with_level(true)
        .with_ansi(false)
        .with_max_level(tracing::Level::TRACE)
        .finish();
    let guard = tracing::subscriber::set_default(subscriber);

    ClientTracingWorld {
        addr: None,
        server: None,
        client: None,
        tracing_config: config,
        captured,
        _subscriber_guard: guard,
    }
}

/// Fixture for `ClientTracingWorld` with default tracing config.
#[rustfmt::skip]
#[fixture]
pub fn client_tracing_world() -> ClientTracingWorld {
    new_world(TracingConfig::default())
}

impl ClientTracingWorld {
    /// Start a standard echo server.
    ///
    /// # Errors
    /// Returns an error if binding fails.
    pub async fn start_echo_server(&mut self) -> TestResult {
        let listener = TcpListener::bind("127.0.0.1:0").await?;
        let addr = listener.local_addr()?;

        let handle = tokio::spawn(async move {
            let Ok((stream, _)) = listener.accept().await else {
                return;
            };
            let mut framed = Framed::new(stream, LengthDelimitedCodec::new());

            while let Some(Ok(bytes)) = framed.next().await {
                let Some(response_bytes) = process_frame(ServerMode::Echo, &bytes) else {
                    break;
                };
                if framed.send(Bytes::from(response_bytes)).await.is_err() {
                    break;
                }
            }
        });

        self.addr = Some(addr);
        self.server = Some(handle);
        Ok(())
    }

    /// Update the tracing config.
    pub fn set_tracing_config(&mut self, config: TracingConfig) { self.tracing_config = config; }

    /// Connect to the server with the current tracing config.
    ///
    /// # Errors
    /// Returns an error if connection fails.
    pub async fn connect(&mut self) -> TestResult {
        let addr = self.addr.ok_or("server address missing")?;
        let client = WireframeClient::builder()
            .tracing_config(self.tracing_config.clone())
            .connect(addr)
            .await?;
        self.client = Some(client);
        Ok(())
    }

    /// Send an envelope via the connected client.
    ///
    /// # Errors
    /// Returns an error if send fails.
    pub async fn send_envelope(&mut self) -> TestResult {
        let client = self.client.as_mut().ok_or("client not connected")?;
        let envelope = Envelope::new(1, None, vec![1, 2, 3]);
        client.send(&envelope).await?;
        Ok(())
    }

    /// Send an envelope and receive the echoed response.
    ///
    /// # Errors
    /// Returns an error if send or receive fails.
    pub async fn send_and_receive(&mut self) -> TestResult {
        let client = self.client.as_mut().ok_or("client not connected")?;
        let envelope = Envelope::new(1, None, vec![1, 2, 3]);
        let _response: Envelope = client.call(&envelope).await?;
        Ok(())
    }

    /// Close the client connection.
    pub async fn close_connection(&mut self) {
        if let Some(client) = self.client.take() {
            client.close().await;
        }
    }

    /// Return the peer address string.
    pub fn peer_addr_string(&self) -> String {
        self.addr.map(|a| a.to_string()).unwrap_or_default()
    }

    /// Check whether captured tracing output contains a needle.
    pub fn output_contains(&self, needle: &str) -> bool {
        let buf = self.captured.lock().expect("lock captured buffer");
        let output = String::from_utf8_lossy(&buf);
        output.contains(needle)
    }

    /// Assert that the output contains the needle.
    ///
    /// # Errors
    /// Returns an error if the needle is not found.
    pub fn assert_output_contains(&self, needle: &str) -> TestResult {
        if self.output_contains(needle) {
            Ok(())
        } else {
            let buf = self.captured.lock().expect("lock captured buffer");
            let output = String::from_utf8_lossy(&buf);
            Err(format!("expected output to contain {needle:?}, got:\n{output}").into())
        }
    }

    /// Assert that the output does NOT contain the needle.
    ///
    /// # Errors
    /// Returns an error if the needle is found.
    pub fn assert_output_not_contains(&self, needle: &str) -> TestResult {
        if self.output_contains(needle) {
            Err(format!("expected output NOT to contain {needle:?}, but it was present").into())
        } else {
            Ok(())
        }
    }
}
