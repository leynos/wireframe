//! `ClientRequestHooksWorld` fixture for rstest-bdd tests.
//!
//! Provides server/client coordination for request hook scenarios.

#![expect(
    clippy::expect_used,
    reason = "test code uses expect for concise assertions"
)]
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

/// Client type alias for request hook tests.
type TestClient = WireframeClient<BincodeSerializer, RewindStream<tokio::net::TcpStream>>;

/// Test world for client request hook scenarios.
#[derive(Debug, Default)]
pub struct ClientRequestHooksWorld {
    addr: Option<SocketAddr>,
    server: Option<JoinHandle<()>>,
    client: Option<TestClient>,
    before_send_count: Arc<AtomicUsize>,
    after_receive_count: Arc<AtomicUsize>,
    marker_log: Arc<Mutex<Vec<u8>>>,
}

impl Drop for ClientRequestHooksWorld {
    fn drop(&mut self) {
        if let Some(handle) = self.server.take() {
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
            let (stream, _) = listener.accept().await.expect("accept client");
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

    /// Connect a client with a `before_send` counter hook.
    ///
    /// # Errors
    /// Returns an error if server address is missing or connection fails.
    pub async fn connect_with_before_send_counter(&mut self) -> TestResult {
        let addr = self.addr.ok_or("server address missing")?;
        let count = Arc::clone(&self.before_send_count);

        let client = WireframeClient::builder()
            .before_send(move |_bytes: &mut Vec<u8>| {
                count.fetch_add(1, Ordering::SeqCst);
            })
            .connect(addr)
            .await?;

        self.client = Some(client);
        Ok(())
    }

    /// Connect a client with an `after_receive` counter hook.
    ///
    /// # Errors
    /// Returns an error if server address is missing or connection fails.
    pub async fn connect_with_after_receive_counter(&mut self) -> TestResult {
        let addr = self.addr.ok_or("server address missing")?;
        let count = Arc::clone(&self.after_receive_count);

        let client = WireframeClient::builder()
            .after_receive(move |_bytes: &mut bytes::BytesMut| {
                count.fetch_add(1, Ordering::SeqCst);
            })
            .connect(addr)
            .await?;

        self.client = Some(client);
        Ok(())
    }

    /// Connect a client with two `before_send` hooks that append markers.
    ///
    /// # Errors
    /// Returns an error if server address is missing or connection fails.
    pub async fn connect_with_marker_hooks(&mut self) -> TestResult {
        let addr = self.addr.ok_or("server address missing")?;
        let log_a = Arc::clone(&self.marker_log);
        let log_b = Arc::clone(&self.marker_log);

        let client = WireframeClient::builder()
            .before_send(move |_bytes: &mut Vec<u8>| {
                log_a.lock().expect("lock").push(b'A');
            })
            .before_send(move |_bytes: &mut Vec<u8>| {
                log_b.lock().expect("lock").push(b'B');
            })
            .connect(addr)
            .await?;

        self.client = Some(client);
        Ok(())
    }

    /// Connect a client with both counter hooks.
    ///
    /// # Errors
    /// Returns an error if server address is missing or connection fails.
    pub async fn connect_with_both_counters(&mut self) -> TestResult {
        let addr = self.addr.ok_or("server address missing")?;
        let sc = Arc::clone(&self.before_send_count);
        let rc = Arc::clone(&self.after_receive_count);

        let client = WireframeClient::builder()
            .before_send(move |_bytes: &mut Vec<u8>| {
                sc.fetch_add(1, Ordering::SeqCst);
            })
            .after_receive(move |_bytes: &mut bytes::BytesMut| {
                rc.fetch_add(1, Ordering::SeqCst);
            })
            .connect(addr)
            .await?;

        self.client = Some(client);
        Ok(())
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

    /// Send and receive an envelope via the configured client.
    ///
    /// # Errors
    /// Returns an error if client is missing or send/receive fails.
    pub async fn send_and_receive_envelope(&mut self) -> TestResult {
        let client = self.client.as_mut().ok_or("client missing")?;
        let envelope = Envelope::new(1, None, vec![1, 2, 3]);
        client.send_envelope(envelope).await?;
        let _response: Envelope = client.receive_envelope().await?;
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

    /// Get the `before_send` counter value.
    #[must_use]
    pub fn before_send_count(&self) -> usize { self.before_send_count.load(Ordering::SeqCst) }

    /// Get the `after_receive` counter value.
    #[must_use]
    pub fn after_receive_count(&self) -> usize { self.after_receive_count.load(Ordering::SeqCst) }

    /// Get the marker log contents.
    #[must_use]
    pub fn marker_log(&self) -> Vec<u8> { self.marker_log.lock().expect("lock").clone() }
}
