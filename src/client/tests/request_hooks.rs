//! Unit tests for client request hooks (`before_send` and `after_receive`).

use std::sync::{
    Arc,
    atomic::{AtomicUsize, Ordering},
};

use bytes::Bytes;
use futures::{SinkExt, StreamExt};
use rstest::{fixture, rstest};
use tokio::net::TcpListener;
use tokio_util::codec::{Framed, LengthDelimitedCodec};
use wireframe_testing::ServerMode;

use crate::{app::Envelope, client::WireframeClient, correlation::CorrelatableFrame};

/// Spawn a simple echo server that reads one frame and echoes it back.
async fn spawn_echo_server() -> (std::net::SocketAddr, tokio::task::JoinHandle<()>) {
    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind listener");
    let addr = listener.local_addr().expect("listener addr");

    let handle = tokio::spawn(async move {
        let (stream, _) = listener.accept().await.expect("accept client");
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
    });

    (addr, handle)
}

/// Spawn a server that captures each received frame payload for inspection.
async fn spawn_capturing_server() -> (std::net::SocketAddr, tokio::task::JoinHandle<Vec<Vec<u8>>>) {
    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind listener");
    let addr = listener.local_addr().expect("listener addr");

    let handle = tokio::spawn(async move {
        let (stream, _) = listener.accept().await.expect("accept client");
        let mut framed = Framed::new(stream, LengthDelimitedCodec::new());
        let mut captured = Vec::new();

        while let Some(Ok(bytes)) = framed.next().await {
            captured.push(bytes.to_vec());
            // Echo the frame back so the client can receive it.
            if framed.send(bytes.freeze()).await.is_err() {
                break;
            }
        }
        captured
    });

    (addr, handle)
}

/// Client type returned by [`connect_client_with_hooks`].
type TestClient = WireframeClient<
    crate::serializer::BincodeSerializer,
    crate::rewind_stream::RewindStream<tokio::net::TcpStream>,
>;

/// Helper to build and connect a client with custom hook configuration.
async fn connect_client_with_hooks<F>(addr: std::net::SocketAddr, configure: F) -> TestClient
where
    F: FnOnce(crate::client::WireframeClientBuilder) -> crate::client::WireframeClientBuilder,
{
    let builder = WireframeClient::builder();
    configure(builder)
        .connect(addr)
        .await
        .expect("connect client")
}

/// Generic test harness for request hook tests.
///
/// Spawns an echo server, connects a client with custom hooks,
/// runs the test body, and handles cleanup automatically.
async fn run_hook_test<F, T>(configure_hooks: F, test_body: T)
where
    F: FnOnce(crate::client::WireframeClientBuilder) -> crate::client::WireframeClientBuilder,
    T: for<'a> FnOnce(
        &'a mut TestClient,
    ) -> std::pin::Pin<Box<dyn std::future::Future<Output = ()> + 'a>>,
{
    let (addr, server) = spawn_echo_server().await;
    let mut client = connect_client_with_hooks(addr, configure_hooks).await;
    test_body(&mut client).await;
    drop(client);
    let _ = server.await;
}

/// Variant for tests that need the capturing server.
async fn run_hook_test_with_capture<F, T>(configure_hooks: F, test_body: T) -> Vec<Vec<u8>>
where
    F: FnOnce(crate::client::WireframeClientBuilder) -> crate::client::WireframeClientBuilder,
    T: for<'a> FnOnce(
        &'a mut TestClient,
    ) -> std::pin::Pin<Box<dyn std::future::Future<Output = ()> + 'a>>,
{
    let (addr, server) = spawn_capturing_server().await;
    let mut client = connect_client_with_hooks(addr, configure_hooks).await;
    test_body(&mut client).await;
    drop(client);
    server.await.expect("server completes")
}

/// Test fixture for tracking hook invocations with a counter.
struct HookCounter {
    counter: Arc<AtomicUsize>,
}

impl HookCounter {
    fn new() -> Self {
        Self {
            counter: Arc::new(AtomicUsize::new(0)),
        }
    }

    fn before_send_hook(&self) -> impl Fn(&mut Vec<u8>) + Send + Sync + 'static {
        let count = self.counter.clone();
        move |_bytes: &mut Vec<u8>| {
            count.fetch_add(1, Ordering::SeqCst);
        }
    }

    fn after_receive_hook(&self) -> impl Fn(&mut bytes::BytesMut) + Send + Sync + 'static {
        let count = self.counter.clone();
        move |_bytes: &mut bytes::BytesMut| {
            count.fetch_add(1, Ordering::SeqCst);
        }
    }

    fn assert_count(&self, expected: usize, message: &str) {
        assert_eq!(self.counter.load(Ordering::SeqCst), expected, "{message}");
    }
}

/// Test fixture for tracking hook invocations with a log.
struct HookLog {
    log: Arc<std::sync::Mutex<Vec<u8>>>,
}

impl HookLog {
    fn new() -> Self {
        Self {
            log: Arc::new(std::sync::Mutex::new(Vec::new())),
        }
    }

    fn before_send_hook(&self, marker: u8) -> impl Fn(&mut Vec<u8>) + Send + Sync + 'static {
        let log = self.log.clone();
        move |_bytes: &mut Vec<u8>| {
            log.lock().expect("lock").push(marker);
        }
    }

    fn after_receive_hook(
        &self,
        marker: u8,
    ) -> impl Fn(&mut bytes::BytesMut) + Send + Sync + 'static {
        let log = self.log.clone();
        move |_bytes: &mut bytes::BytesMut| {
            log.lock().expect("lock").push(marker);
        }
    }

    fn assert_entries(&self, expected: &[u8], message: &str) {
        assert_eq!(
            self.log.lock().expect("lock").as_slice(),
            expected,
            "{message}"
        );
    }
}

/// Fixture: create a fresh [`HookCounter`].
#[rustfmt::skip]
#[fixture]
fn hook_counter() -> HookCounter {
    HookCounter::new()
}

/// Fixture: create a fresh [`HookLog`].
#[rustfmt::skip]
#[fixture]
fn hook_log() -> HookLog {
    HookLog::new()
}

/// Helper: test body that sends an envelope.
async fn send_envelope_test_body(client: &mut TestClient) {
    let envelope = Envelope::new(1, None, vec![1, 2, 3]);
    client.send_envelope(envelope).await.expect("send envelope");
}

/// Helper: test body that sends an envelope and receives a response.
async fn send_and_receive_test_body(client: &mut TestClient) {
    let envelope = Envelope::new(1, None, vec![1, 2, 3]);
    client.send_envelope(envelope).await.expect("send envelope");
    let _response: Envelope = client.receive_envelope().await.expect("receive envelope");
}

#[rstest]
#[tokio::test]
async fn before_send_hook_invoked_on_send(hook_counter: HookCounter) {
    run_hook_test(
        |b| b.before_send(hook_counter.before_send_hook()),
        |client| Box::pin(send_envelope_test_body(client)),
    )
    .await;

    hook_counter.assert_count(1, "before_send hook should be invoked once");
}

#[rstest]
#[tokio::test]
async fn after_receive_hook_invoked_on_receive(hook_counter: HookCounter) {
    run_hook_test(
        |b| b.after_receive(hook_counter.after_receive_hook()),
        |client| Box::pin(send_and_receive_test_body(client)),
    )
    .await;

    hook_counter.assert_count(1, "after_receive hook should be invoked once");
}

#[rstest]
#[case::before_send(true)]
#[case::after_receive(false)]
#[tokio::test]
async fn multiple_hooks_execute_in_registration_order(
    hook_log: HookLog,
    #[case] is_before_send: bool,
) {
    run_hook_test(
        |b| {
            if is_before_send {
                b.before_send(hook_log.before_send_hook(b'A'))
                    .before_send(hook_log.before_send_hook(b'B'))
            } else {
                b.after_receive(hook_log.after_receive_hook(b'A'))
                    .after_receive(hook_log.after_receive_hook(b'B'))
            }
        },
        |client| {
            if is_before_send {
                Box::pin(send_envelope_test_body(client))
            } else {
                Box::pin(send_and_receive_test_body(client))
            }
        },
    )
    .await;

    hook_log.assert_entries(b"AB", "hooks should execute in registration order");
}

#[tokio::test]
async fn both_hooks_fire_for_call_correlated() {
    let send_counter = HookCounter::new();
    let recv_counter = HookCounter::new();

    run_hook_test(
        |b| {
            b.before_send(send_counter.before_send_hook())
                .after_receive(recv_counter.after_receive_hook())
        },
        |client| {
            Box::pin(async move {
                let request = Envelope::new(1, None, vec![10, 20]);
                let _response: Envelope = client.call_correlated(request).await.expect("call");
            })
        },
    )
    .await;

    send_counter.assert_count(1, "before_send fires");
    recv_counter.assert_count(1, "after_receive fires");
}

#[tokio::test]
async fn no_hooks_configured_works_identically() {
    let correlation_id = Arc::new(std::sync::Mutex::new(None));
    let cid = correlation_id.clone();

    run_hook_test(
        |b| b,
        |client| {
            Box::pin(async move {
                let request = Envelope::new(1, None, vec![5, 6, 7]);
                let response: Envelope = client.call_correlated(request).await.expect("call");
                *cid.lock().expect("lock") = response.correlation_id();
            })
        },
    )
    .await;

    assert_eq!(
        *correlation_id.lock().expect("lock"),
        Some(1),
        "correlation ID should match without hooks"
    );
}

#[derive(bincode::Encode, bincode::BorrowDecode)]
struct Ping(u8);

#[rstest]
#[tokio::test]
async fn before_send_hook_fires_for_plain_send(hook_counter: HookCounter) {
    let _captured = run_hook_test_with_capture(
        |b| b.before_send(hook_counter.before_send_hook()),
        |client| {
            Box::pin(async move {
                // Use the plain send() API (not envelope-aware).
                client.send(&Ping(42)).await.expect("send");
            })
        },
    )
    .await;

    hook_counter.assert_count(1, "before_send should fire for plain send()");
}

#[tokio::test]
async fn before_send_hook_can_mutate_frame_bytes_on_wire() {
    const MARKER: u8 = 0xff;

    let captured = run_hook_test_with_capture(
        |b| {
            b.before_send(move |bytes: &mut Vec<u8>| {
                bytes.push(MARKER);
            })
        },
        |client| Box::pin(send_envelope_test_body(client)),
    )
    .await;

    let frame = captured
        .first()
        .expect("server should capture exactly one frame");
    assert_eq!(
        frame.last().copied(),
        Some(MARKER),
        "marker byte appended by before_send hook should be visible on the wire"
    );
}

#[tokio::test]
async fn after_receive_hook_can_mutate_frame_bytes_before_deserialization() {
    // Pre-serialize a replacement envelope with a distinctive payload.
    let replacement = Envelope::new(42, Some(1), vec![99, 98, 97]);
    let replacement_bytes =
        bincode::encode_to_vec(&replacement, bincode::config::standard()).expect("encode");

    let hook = move |bytes: &mut bytes::BytesMut| {
        bytes.clear();
        bytes.extend_from_slice(&replacement_bytes);
    };

    run_hook_test(
        |b| b.after_receive(hook),
        |client| {
            Box::pin(async move {
                let envelope = Envelope::new(1, None, vec![1, 2, 3]);
                client.send_envelope(envelope).await.expect("send envelope");
                let response: Envelope = client.receive_envelope().await.expect("receive envelope");
                assert_eq!(
                    response.payload_bytes(),
                    &[99, 98, 97],
                    "after_receive hook mutation should be reflected in deserialized envelope"
                );
            })
        },
    )
    .await;
}
