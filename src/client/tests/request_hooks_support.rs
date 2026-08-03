//! Shared support for the client request-hook tests.
//!
//! Holds the echo and capturing servers, the hook harnesses, and the fixtures
//! that [`super::request_hooks`] exercises, keeping that module within the
//! repository's 400-line limit.

use std::sync::{
    Arc,
    Mutex,
    MutexGuard,
    PoisonError,
    atomic::{AtomicUsize, Ordering},
};

use bytes::Bytes;
use futures::{SinkExt, StreamExt};
use rstest::fixture;
use tokio_util::codec::{Framed, LengthDelimitedCodec};
use wireframe_testing::ServerMode;

use super::helpers::{TestResult, bind_loopback};
use crate::{app::Envelope, client::WireframeClient};

/// Spawn a simple echo server that reads one frame and echoes it back.
pub(super) async fn spawn_echo_server() -> TestResult<(
    std::net::SocketAddr,
    tokio::task::JoinHandle<std::io::Result<()>>,
)> {
    let (listener, addr) = bind_loopback().await?;

    let handle = tokio::spawn(async move {
        let (stream, _) = listener.accept().await?;
        let mut framed = Framed::new(stream, LengthDelimitedCodec::new());

        while let Some(Ok(bytes)) = framed.next().await {
            let Some(response_bytes) = wireframe_testing::process_frame(ServerMode::Echo, &bytes)
            else {
                break;
            };
            if framed.send(Bytes::from(response_bytes)).await.is_err() {
                break;
            }
        }
        Ok(())
    });

    Ok((addr, handle))
}

/// Spawn a server that captures each received frame payload for inspection.
pub(super) async fn spawn_capturing_server() -> TestResult<(
    std::net::SocketAddr,
    tokio::task::JoinHandle<std::io::Result<Vec<Vec<u8>>>>,
)> {
    let (listener, addr) = bind_loopback().await?;

    let handle = tokio::spawn(async move {
        let (stream, _) = listener.accept().await?;
        let mut framed = Framed::new(stream, LengthDelimitedCodec::new());
        let mut captured = Vec::new();

        while let Some(Ok(bytes)) = framed.next().await {
            captured.push(bytes.to_vec());
            // Echo the frame back so the client can receive it.
            if framed.send(bytes.freeze()).await.is_err() {
                break;
            }
        }
        Ok(captured)
    });

    Ok((addr, handle))
}

/// Locks a hook log, recovering the entries if a previous holder panicked.
///
/// The log only accumulates marker bytes, so no invariant is broken by a
/// poisoning; discarding the recorded markers would lose the very evidence the
/// assertion needs.
pub(super) fn lock_log(log: &Mutex<Vec<u8>>) -> MutexGuard<'_, Vec<u8>> {
    log.lock().unwrap_or_else(PoisonError::into_inner)
}

/// Client type returned by [`connect_client_with_hooks`].
pub(super) type TestClient = WireframeClient<
    crate::serializer::BincodeSerializer,
    crate::rewind_stream::RewindStream<tokio::net::TcpStream>,
>;

/// Helper to build and connect a client with custom hook configuration.
pub(super) async fn connect_client_with_hooks<F>(
    addr: std::net::SocketAddr,
    configure: F,
) -> TestResult<TestClient>
where
    F: FnOnce(crate::client::WireframeClientBuilder) -> crate::client::WireframeClientBuilder,
{
    let builder = WireframeClient::builder();
    Ok(configure(builder).connect(addr).await?)
}

/// Generic test harness for request hook tests.
///
/// Spawns an echo server, connects a client with custom hooks,
/// runs the test body, and handles cleanup automatically.
pub(super) async fn run_hook_test<F, T>(configure_hooks: F, test_body: T) -> TestResult
where
    F: FnOnce(crate::client::WireframeClientBuilder) -> crate::client::WireframeClientBuilder,
    T: for<'a> FnOnce(
        &'a mut TestClient,
    ) -> std::pin::Pin<Box<dyn std::future::Future<Output = TestResult> + 'a>>,
{
    let (addr, server) = spawn_echo_server().await?;
    let mut client = connect_client_with_hooks(addr, configure_hooks).await?;
    test_body(&mut client).await?;
    drop(client);
    let _ = server.await;
    Ok(())
}

/// Variant for tests that need the capturing server.
pub(super) async fn run_hook_test_with_capture<F, T>(
    configure_hooks: F,
    test_body: T,
) -> TestResult<Vec<Vec<u8>>>
where
    F: FnOnce(crate::client::WireframeClientBuilder) -> crate::client::WireframeClientBuilder,
    T: for<'a> FnOnce(
        &'a mut TestClient,
    ) -> std::pin::Pin<Box<dyn std::future::Future<Output = TestResult> + 'a>>,
{
    let (addr, server) = spawn_capturing_server().await?;
    let mut client = connect_client_with_hooks(addr, configure_hooks).await?;
    test_body(&mut client).await?;
    drop(client);
    Ok(server.await??)
}

/// Test fixture for tracking hook invocations with a counter.
pub(super) struct HookCounter {
    counter: Arc<AtomicUsize>,
}

impl HookCounter {
    pub(super) fn new() -> Self {
        Self {
            counter: Arc::new(AtomicUsize::new(0)),
        }
    }

    pub(super) fn before_send_hook(&self) -> impl Fn(&mut Vec<u8>) + Send + Sync + 'static {
        let count = self.counter.clone();
        move |_bytes: &mut Vec<u8>| {
            count.fetch_add(1, Ordering::SeqCst);
        }
    }

    pub(super) fn after_receive_hook(
        &self,
    ) -> impl Fn(&mut bytes::BytesMut) + Send + Sync + 'static {
        let count = self.counter.clone();
        move |_bytes: &mut bytes::BytesMut| {
            count.fetch_add(1, Ordering::SeqCst);
        }
    }

    pub(super) fn assert_count(&self, expected: usize, message: &str) {
        assert_eq!(self.counter.load(Ordering::SeqCst), expected, "{message}");
    }
}

/// Test fixture for tracking hook invocations with a log.
pub(super) struct HookLog {
    log: Arc<Mutex<Vec<u8>>>,
}

impl HookLog {
    pub(super) fn new() -> Self {
        Self {
            log: Arc::new(Mutex::new(Vec::new())),
        }
    }

    pub(super) fn before_send_hook(
        &self,
        marker: u8,
    ) -> impl Fn(&mut Vec<u8>) + Send + Sync + 'static {
        let log = self.log.clone();
        move |_bytes: &mut Vec<u8>| {
            lock_log(&log).push(marker);
        }
    }

    pub(super) fn after_receive_hook(
        &self,
        marker: u8,
    ) -> impl Fn(&mut bytes::BytesMut) + Send + Sync + 'static {
        let log = self.log.clone();
        move |_bytes: &mut bytes::BytesMut| {
            lock_log(&log).push(marker);
        }
    }

    pub(super) fn assert_entries(&self, expected: &[u8], message: &str) {
        assert_eq!(lock_log(&self.log).as_slice(), expected, "{message}");
    }
}

/// Fixture: create a fresh [`HookCounter`].
#[rustfmt::skip]
#[fixture]
pub(super) fn hook_counter() -> HookCounter {
    HookCounter::new()
}

/// Fixture: create a fresh [`HookLog`].
#[rustfmt::skip]
#[fixture]
pub(super) fn hook_log() -> HookLog {
    HookLog::new()
}

/// Helper: test body that sends an envelope.
pub(super) async fn send_envelope_test_body(client: &mut TestClient) -> TestResult {
    let envelope = Envelope::new(1, None, vec![1, 2, 3]);
    client.send_envelope(envelope).await?;
    Ok(())
}

/// Helper: test body that sends an envelope and receives a response.
pub(super) async fn send_and_receive_test_body(client: &mut TestClient) -> TestResult {
    let envelope = Envelope::new(1, None, vec![1, 2, 3]);
    client.send_envelope(envelope).await?;
    let _response: Envelope = client.receive_envelope().await?;
    Ok(())
}
