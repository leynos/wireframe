//! Unit tests for client request hooks (`before_send` and `after_receive`).

use std::sync::{
    Arc,
    atomic::{AtomicUsize, Ordering},
};

use bytes::Bytes;
use futures::{SinkExt, StreamExt};
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

#[tokio::test]
async fn before_send_hook_invoked_on_send() {
    let counter = Arc::new(AtomicUsize::new(0));
    let count = counter.clone();

    let (addr, server) = spawn_echo_server().await;

    let mut client = connect_client_with_hooks(addr, |b| {
        b.before_send(move |_bytes: &mut Vec<u8>| {
            count.fetch_add(1, Ordering::SeqCst);
        })
    })
    .await;

    let envelope = Envelope::new(1, None, vec![1, 2, 3]);
    client.send_envelope(envelope).await.expect("send envelope");

    assert_eq!(
        counter.load(Ordering::SeqCst),
        1,
        "before_send hook should be invoked once"
    );

    drop(client);
    let _ = server.await;
}

#[tokio::test]
async fn after_receive_hook_invoked_on_receive() {
    let counter = Arc::new(AtomicUsize::new(0));
    let count = counter.clone();

    let (addr, server) = spawn_echo_server().await;

    let mut client = connect_client_with_hooks(addr, |b| {
        b.after_receive(move |_bytes: &mut bytes::BytesMut| {
            count.fetch_add(1, Ordering::SeqCst);
        })
    })
    .await;

    let envelope = Envelope::new(1, None, vec![1, 2, 3]);
    client.send_envelope(envelope).await.expect("send envelope");

    let _response: Envelope = client.receive_envelope().await.expect("receive envelope");

    assert_eq!(
        counter.load(Ordering::SeqCst),
        1,
        "after_receive hook should be invoked once"
    );

    drop(client);
    let _ = server.await;
}

#[tokio::test]
async fn multiple_before_send_hooks_execute_in_order() {
    let log = Arc::new(std::sync::Mutex::new(Vec::<u8>::new()));
    let log_a = log.clone();
    let log_b = log.clone();

    let (addr, server) = spawn_echo_server().await;

    let mut client = connect_client_with_hooks(addr, |b| {
        b.before_send(move |_bytes: &mut Vec<u8>| {
            log_a.lock().expect("lock").push(b'A');
        })
        .before_send(move |_bytes: &mut Vec<u8>| {
            log_b.lock().expect("lock").push(b'B');
        })
    })
    .await;

    let envelope = Envelope::new(1, None, vec![1, 2, 3]);
    client.send_envelope(envelope).await.expect("send envelope");

    {
        let entries = log.lock().expect("lock");
        assert_eq!(
            entries.as_slice(),
            b"AB",
            "hooks should execute in registration order"
        );
    }

    drop(client);
    let _ = server.await;
}

#[tokio::test]
async fn multiple_after_receive_hooks_execute_in_order() {
    let log = Arc::new(std::sync::Mutex::new(Vec::<u8>::new()));
    let log_a = log.clone();
    let log_b = log.clone();

    let (addr, server) = spawn_echo_server().await;

    let mut client = connect_client_with_hooks(addr, |b| {
        b.after_receive(move |_bytes: &mut bytes::BytesMut| {
            log_a.lock().expect("lock").push(b'A');
        })
        .after_receive(move |_bytes: &mut bytes::BytesMut| {
            log_b.lock().expect("lock").push(b'B');
        })
    })
    .await;

    let envelope = Envelope::new(1, None, vec![1, 2, 3]);
    client.send_envelope(envelope).await.expect("send envelope");
    let _response: Envelope = client.receive_envelope().await.expect("receive envelope");

    {
        let entries = log.lock().expect("lock");
        assert_eq!(
            entries.as_slice(),
            b"AB",
            "hooks should execute in registration order"
        );
    }

    drop(client);
    let _ = server.await;
}

#[tokio::test]
async fn both_hooks_fire_for_call_correlated() {
    let send_count = Arc::new(AtomicUsize::new(0));
    let recv_count = Arc::new(AtomicUsize::new(0));
    let sc = send_count.clone();
    let rc = recv_count.clone();

    let (addr, server) = spawn_echo_server().await;

    let mut client = connect_client_with_hooks(addr, |b| {
        b.before_send(move |_bytes: &mut Vec<u8>| {
            sc.fetch_add(1, Ordering::SeqCst);
        })
        .after_receive(move |_bytes: &mut bytes::BytesMut| {
            rc.fetch_add(1, Ordering::SeqCst);
        })
    })
    .await;

    let request = Envelope::new(1, None, vec![10, 20]);
    let _response: Envelope = client.call_correlated(request).await.expect("call");

    assert_eq!(send_count.load(Ordering::SeqCst), 1, "before_send fires");
    assert_eq!(recv_count.load(Ordering::SeqCst), 1, "after_receive fires");

    drop(client);
    let _ = server.await;
}

#[tokio::test]
async fn no_hooks_configured_works_identically() {
    let (addr, server) = spawn_echo_server().await;

    let mut client = connect_client_with_hooks(addr, |b| b).await;

    let request = Envelope::new(1, None, vec![5, 6, 7]);
    let response: Envelope = client.call_correlated(request).await.expect("call");

    assert_eq!(
        response.correlation_id(),
        Some(1),
        "correlation ID should match without hooks"
    );

    drop(client);
    let _ = server.await;
}

#[derive(bincode::Encode, bincode::BorrowDecode)]
struct Ping(u8);

#[tokio::test]
async fn before_send_hook_fires_for_plain_send() {
    let counter = Arc::new(AtomicUsize::new(0));
    let count = counter.clone();

    let (addr, server) = spawn_capturing_server().await;

    let mut client = connect_client_with_hooks(addr, |b| {
        b.before_send(move |_bytes: &mut Vec<u8>| {
            count.fetch_add(1, Ordering::SeqCst);
        })
    })
    .await;

    // Use the plain send() API (not envelope-aware).
    client.send(&Ping(42)).await.expect("send");

    assert_eq!(
        counter.load(Ordering::SeqCst),
        1,
        "before_send should fire for plain send()"
    );

    drop(client);
    let _ = server.await;
}
