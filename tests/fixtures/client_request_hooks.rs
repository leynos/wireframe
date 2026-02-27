//! `ClientRequestHooksWorld` fixture for rstest-bdd tests.
//!
//! Provides server/client coordination for request hook scenarios.

#![expect(
    clippy::excessive_nesting,
    reason = "spawned server tasks naturally nest several levels deep"
)]

use std::{
    net::SocketAddr,
    sync::{
        Arc,
        Mutex,
        atomic::{AtomicUsize, Ordering},
    },
};

use bytes::Bytes;
use futures::{SinkExt, StreamExt};
use rstest::fixture;
use tokio::{net::TcpListener, task::JoinHandle};
use tokio_util::codec::{Framed, LengthDelimitedCodec};
use wireframe::{
    app::Envelope,
    client::WireframeClient,
    rewind_stream::RewindStream,
    serializer::BincodeSerializer,
};
/// Re-export `TestResult` from `wireframe_testing` for use in steps.
pub use wireframe_testing::TestResult;
use wireframe_testing::{ServerMode, process_frame};

/// Marker byte appended by the `before_send` mutation hook.
const MARKER_BYTE: u8 = 0xff;

/// Client type alias for request hook tests.
type TestClient = WireframeClient<BincodeSerializer, RewindStream<tokio::net::TcpStream>>;

/// Test world for client request hook scenarios.
#[derive(Debug, Default)]
pub struct ClientRequestHooksWorld {
    addr: Option<SocketAddr>,
    server: Option<JoinHandle<()>>,
    capturing_server: Option<JoinHandle<Vec<Vec<u8>>>>,
    client: Option<TestClient>,
    before_send_count: Arc<AtomicUsize>,
    after_receive_count: Arc<AtomicUsize>,
    marker_log: Arc<Mutex<Vec<u8>>>,
    received_payload: Option<Vec<u8>>,
}

impl Drop for ClientRequestHooksWorld {
    fn drop(&mut self) {
        if let Some(handle) = self.server.take() {
            handle.abort();
        }
        if let Some(handle) = self.capturing_server.take() {
            handle.abort();
        }
    }
}

/// Fixture for `ClientRequestHooksWorld`.
#[rustfmt::skip]
#[fixture]
pub fn client_request_hooks_world() -> ClientRequestHooksWorld {
    ClientRequestHooksWorld::default()
}

impl ClientRequestHooksWorld {
    /// Spawn an echo server that echoes every frame back.
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

    /// Spawn a capturing server that records raw frames and echoes them back.
    ///
    /// # Errors
    /// Returns an error if binding fails.
    pub async fn start_capturing_server(&mut self) -> TestResult {
        let listener = TcpListener::bind("127.0.0.1:0").await?;
        let addr = listener.local_addr()?;

        let handle = tokio::spawn(async move {
            let Ok((stream, _)) = listener.accept().await else {
                return Vec::new();
            };
            let mut framed = Framed::new(stream, LengthDelimitedCodec::new());
            let mut captured = Vec::new();

            while let Some(Ok(bytes)) = framed.next().await {
                captured.push(bytes.to_vec());
                if framed.send(bytes.freeze()).await.is_err() {
                    break;
                }
            }
            captured
        });

        self.addr = Some(addr);
        self.capturing_server = Some(handle);
        Ok(())
    }

    /// Connect a client with custom builder configuration.
    ///
    /// # Errors
    /// Returns an error if server address is missing or connection fails.
    async fn connect_with_hooks<F>(&mut self, configure: F) -> TestResult
    where
        F: FnOnce(
            wireframe::client::WireframeClientBuilder,
        ) -> wireframe::client::WireframeClientBuilder,
    {
        let addr = self.addr.ok_or("server address missing")?;
        let client = configure(WireframeClient::builder()).connect(addr).await?;
        self.client = Some(client);
        Ok(())
    }

    /// Connect a client with a `before_send` counter hook.
    ///
    /// # Errors
    /// Returns an error if server address is missing or connection fails.
    pub async fn connect_with_before_send_counter(&mut self) -> TestResult {
        let count = Arc::clone(&self.before_send_count);
        self.connect_with_hooks(|b| {
            b.before_send(move |_bytes: &mut Vec<u8>| {
                count.fetch_add(1, Ordering::SeqCst);
            })
        })
        .await
    }

    /// Connect a client with an `after_receive` counter hook.
    ///
    /// # Errors
    /// Returns an error if server address is missing or connection fails.
    pub async fn connect_with_after_receive_counter(&mut self) -> TestResult {
        let count = Arc::clone(&self.after_receive_count);
        self.connect_with_hooks(|b| {
            b.after_receive(move |_bytes: &mut bytes::BytesMut| {
                count.fetch_add(1, Ordering::SeqCst);
            })
        })
        .await
    }

    /// Connect a client with two `before_send` hooks that append markers.
    ///
    /// # Errors
    /// Returns an error if server address is missing or connection fails.
    pub async fn connect_with_marker_hooks(&mut self) -> TestResult {
        let log_a = Arc::clone(&self.marker_log);
        let log_b = Arc::clone(&self.marker_log);
        self.connect_with_hooks(|b| {
            b.before_send(move |_bytes: &mut Vec<u8>| {
                if let Ok(mut guard) = log_a.lock() {
                    guard.push(b'A');
                }
            })
            .before_send(move |_bytes: &mut Vec<u8>| {
                if let Ok(mut guard) = log_b.lock() {
                    guard.push(b'B');
                }
            })
        })
        .await
    }

    /// Connect a client with both counter hooks.
    ///
    /// # Errors
    /// Returns an error if server address is missing or connection fails.
    pub async fn connect_with_both_counters(&mut self) -> TestResult {
        let sc = Arc::clone(&self.before_send_count);
        let rc = Arc::clone(&self.after_receive_count);
        self.connect_with_hooks(|b| {
            b.before_send(move |_bytes: &mut Vec<u8>| {
                sc.fetch_add(1, Ordering::SeqCst);
            })
            .after_receive(move |_bytes: &mut bytes::BytesMut| {
                rc.fetch_add(1, Ordering::SeqCst);
            })
        })
        .await
    }

    /// Connect a client with a `before_send` hook that appends a marker byte.
    ///
    /// # Errors
    /// Returns an error if server address is missing or connection fails.
    pub async fn connect_with_before_send_marker(&mut self) -> TestResult {
        self.connect_with_hooks(|b| {
            b.before_send(move |bytes: &mut Vec<u8>| {
                bytes.push(MARKER_BYTE);
            })
        })
        .await
    }

    /// Connect a client with an `after_receive` hook that replaces the frame.
    ///
    /// # Errors
    /// Returns an error if server address is missing or connection fails.
    pub async fn connect_with_after_receive_replacement(&mut self) -> TestResult {
        let replacement = Envelope::new(42, Some(1), vec![99, 98, 97]);
        let replacement_bytes = bincode::encode_to_vec(&replacement, bincode::config::standard())?;

        self.connect_with_hooks(move |b| {
            b.after_receive(move |bytes: &mut bytes::BytesMut| {
                bytes.clear();
                bytes.extend_from_slice(&replacement_bytes);
            })
        })
        .await
    }

    /// Send an envelope via the configured client.
    ///
    /// # Errors
    /// Returns an error if client is missing or send fails.
    pub async fn send_envelope(&mut self) -> TestResult {
        let client = self.client.as_mut().ok_or("client missing")?;
        let envelope = Envelope::new(1, None, vec![1, 2, 3]);
        client.send_envelope(envelope).await?;
        Ok(())
    }

    /// Send and receive an envelope, storing the received payload.
    ///
    /// # Errors
    /// Returns an error if client is missing or send/receive fails.
    pub async fn send_and_receive_envelope(&mut self) -> TestResult {
        let client = self.client.as_mut().ok_or("client missing")?;
        let envelope = Envelope::new(1, None, vec![1, 2, 3]);
        client.send_envelope(envelope).await?;
        let response: Envelope = client.receive_envelope().await?;
        self.received_payload = Some(response.payload_bytes().to_vec());
        Ok(())
    }

    /// Perform a correlated call via the configured client.
    ///
    /// # Errors
    /// Returns an error if client is missing or call fails.
    pub async fn perform_correlated_call(&mut self) -> TestResult {
        let client = self.client.as_mut().ok_or("client missing")?;
        let request = Envelope::new(1, None, vec![10, 20]);
        let _response: Envelope = client.call_correlated(request).await?;
        Ok(())
    }

    /// Collect captured frames from the capturing server.
    ///
    /// # Errors
    /// Returns an error if the capturing server is missing or panicked.
    pub async fn collect_captured_frames(
        &mut self,
    ) -> Result<Vec<Vec<u8>>, Box<dyn std::error::Error + Send + Sync>> {
        // Drop client first so the server sees EOF.
        self.client.take();
        let handle = self
            .capturing_server
            .take()
            .ok_or("capturing server missing")?;
        let frames = handle.await?;
        Ok(frames)
    }

    /// Get the `before_send` counter value.
    #[must_use]
    pub fn before_send_count(&self) -> usize { self.before_send_count.load(Ordering::SeqCst) }

    /// Get the `after_receive` counter value.
    #[must_use]
    pub fn after_receive_count(&self) -> usize { self.after_receive_count.load(Ordering::SeqCst) }

    /// Get the marker log contents.
    ///
    /// # Errors
    /// Returns an error if the lock is poisoned.
    pub fn marker_log(&self) -> Result<Vec<u8>, Box<dyn std::error::Error + Send + Sync>> {
        let guard = self.marker_log.lock().map_err(|e| e.to_string())?;
        Ok(guard.clone())
    }

    /// Get the received payload from the last `send_and_receive_envelope`.
    #[must_use]
    pub fn received_payload(&self) -> Option<&[u8]> { self.received_payload.as_deref() }

    /// The marker byte used by the `before_send` mutation hook.
    #[must_use]
    pub fn marker_byte() -> u8 { MARKER_BYTE }
}
