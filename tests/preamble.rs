#![cfg(not(loom))]
//! Tests for connection preamble reading.

use std::{
    io,
    sync::{Arc, Mutex},
};

use bincode::error::DecodeError;
use futures::future::BoxFuture;
mod common;
use common::{factory, unused_listener};
use rstest::rstest;
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt, duplex},
    net::TcpStream,
    sync::oneshot,
    time::{Duration, timeout},
};
use wireframe::{app::WireframeApp, preamble::read_preamble, server::WireframeServer};

#[derive(Debug, Clone, PartialEq, Eq, bincode::Encode, bincode::Decode)]
struct HotlinePreamble {
    /// Should always be `b"TRTPHOTL"`.
    magic: [u8; 8],
    /// Minimum server version this client supports.
    min_version: u16,
    /// Client version.
    client_version: u16,
}

impl HotlinePreamble {
    const MAGIC: [u8; 8] = *b"TRTPHOTL";

    fn validate(&self) -> Result<(), DecodeError> {
        if self.magic != Self::MAGIC {
            return Err(DecodeError::Other("invalid hotline preamble"));
        }
        Ok(())
    }
}

/// Create a server configured with `HotlinePreamble` handlers.
fn server_with_handlers<F, S, E>(
    factory: F,
    success: S,
    failure: E,
) -> WireframeServer<F, HotlinePreamble>
where
    F: Fn() -> WireframeApp + Send + Sync + Clone + 'static,
    S: for<'a> Fn(&'a HotlinePreamble, &'a mut TcpStream) -> BoxFuture<'a, io::Result<()>>
        + Send
        + Sync
        + 'static,
    E: Fn(&DecodeError) + Send + Sync + 'static,
{
    WireframeServer::new(factory)
        .workers(1)
        .with_preamble::<HotlinePreamble>()
        .on_preamble_decode_success(success)
        .on_preamble_decode_failure(failure)
}

/// Run the provided server while executing `block`.
async fn with_running_server<F, T, Fut, B>(server: WireframeServer<F, T>, block: B)
where
    F: Fn() -> WireframeApp + Send + Sync + Clone + 'static,
    T: wireframe::preamble::Preamble,
    Fut: std::future::Future<Output = ()>,
    B: FnOnce(std::net::SocketAddr) -> Fut,
{
    let listener = unused_listener();
    let server = server.bind_existing_listener(listener).expect("bind");
    let addr = server.local_addr().expect("addr");
    let (shutdown_tx, shutdown_rx) = oneshot::channel::<()>();
    let handle = tokio::spawn(async move {
        server
            .run_with_shutdown(async {
                let _ = shutdown_rx.await;
            })
            .await
            .expect("server run failed");
    });

    block(addr).await;
    let _ = shutdown_tx.send(());
    handle.await.expect("server join failed");
}

#[tokio::test]
async fn parse_valid_preamble() {
    let (mut client, mut server) = duplex(64);
    let bytes = b"TRTPHOTL\x00\x01\x00\x02";
    client.write_all(bytes).await.expect("write failed");
    client.shutdown().await.expect("shutdown failed");
    let (p, _) = read_preamble::<_, HotlinePreamble>(&mut server)
        .await
        .expect("valid preamble");
    eprintln!("decoded: {p:?}");
    p.validate().expect("preamble validation failed");
    assert_eq!(p.magic, HotlinePreamble::MAGIC);
    assert_eq!(p.min_version, 1);
    assert_eq!(p.client_version, 2);
}

#[tokio::test]
async fn invalid_magic_is_error() {
    let (mut client, mut server) = duplex(64);
    let bytes = b"WRONGMAG\x00\x01\x00\x02";
    client.write_all(bytes).await.expect("write failed");
    client.shutdown().await.expect("shutdown failed");
    let (preamble, _) = read_preamble::<_, HotlinePreamble>(&mut server)
        .await
        .expect("decoded");
    assert!(preamble.validate().is_err());
}

#[derive(Clone, Copy)]
enum ExpectedCallback {
    Success,
    Failure,
}

#[rstest]
#[case(b"TRTPHOTL\x00\x01\x00\x02", ExpectedCallback::Success)]
#[case(b"TRTPHOT", ExpectedCallback::Failure)]
#[tokio::test]
async fn server_triggers_expected_callback(
    factory: impl Fn() -> WireframeApp + Send + Sync + Clone + 'static,
    #[case] bytes: &'static [u8],
    #[case] expected: ExpectedCallback,
) {
    let (success_tx, success_rx) = tokio::sync::oneshot::channel::<HotlinePreamble>();
    let (failure_tx, failure_rx) = tokio::sync::oneshot::channel::<()>();
    let success_tx = std::sync::Arc::new(std::sync::Mutex::new(Some(success_tx)));
    let failure_tx = std::sync::Arc::new(std::sync::Mutex::new(Some(failure_tx)));
    let server = server_with_handlers(
        factory,
        {
            let success_tx = success_tx.clone();
            move |p, _| {
                let success_tx = success_tx.clone();
                let clone = p.clone();
                Box::pin(async move {
                    if let Some(tx) = success_tx.lock().expect("lock poisoned").take() {
                        let _ = tx.send(clone);
                    }
                    Ok(())
                })
            }
        },
        {
            let failure_tx = failure_tx.clone();
            move |_| {
                if let Some(tx) = failure_tx.lock().expect("lock poisoned").take() {
                    let _ = tx.send(());
                }
            }
        },
    );

    with_running_server(server, |addr| async move {
        let mut stream = TcpStream::connect(addr).await.expect("connect failed");
        stream.write_all(bytes).await.expect("write failed");
        stream.shutdown().await.expect("shutdown failed");
    })
    .await;

    match expected {
        ExpectedCallback::Success => {
            let preamble = timeout(Duration::from_secs(1), success_rx)
                .await
                .expect("timeout waiting for success")
                .expect("success send");
            assert_eq!(preamble.magic, HotlinePreamble::MAGIC);
            assert!(
                timeout(Duration::from_millis(500), failure_rx)
                    .await
                    .is_err()
            );
        }
        ExpectedCallback::Failure => {
            timeout(Duration::from_secs(1), failure_rx)
                .await
                .expect("timeout waiting for failure")
                .expect("failure send");
            assert!(
                timeout(Duration::from_millis(500), success_rx)
                    .await
                    .is_err()
            );
        }
    }
}

#[rstest]
#[tokio::test]
async fn success_callback_can_write_response(
    factory: impl Fn() -> WireframeApp + Send + Sync + Clone + 'static,
) {
    let server = server_with_handlers(
        factory,
        |_, stream| {
            Box::pin(async move {
                stream.write_all(b"ACK").await.expect("write failed");
                stream.flush().await.expect("flush failed");
                Ok(())
            })
        },
        |_| {},
    );

    with_running_server(server, |addr| async move {
        let mut stream = TcpStream::connect(addr).await.expect("connect failed");
        let bytes = b"TRTPHOTL\x00\x01\x00\x02";
        stream.write_all(bytes).await.expect("write failed");
        let mut buf = [0u8; 3];
        stream.read_exact(&mut buf).await.expect("read failed");
        assert_eq!(&buf, b"ACK");
    })
    .await;
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, bincode::Encode, bincode::Decode)]
struct OtherPreamble(u8);

type Holder = Arc<Mutex<Option<oneshot::Sender<()>>>>;

fn channel_holder() -> (Holder, oneshot::Receiver<()>) {
    let (tx, rx) = oneshot::channel();
    (Arc::new(Mutex::new(Some(tx))), rx)
}

fn success_cb<P>(
    holder: Arc<Mutex<Option<oneshot::Sender<()>>>>,
) -> impl for<'a> Fn(&'a P, &'a mut TcpStream) -> BoxFuture<'a, io::Result<()>> + Send + Sync + 'static
{
    move |_, _| {
        let holder = holder.clone();
        Box::pin(async move {
            if let Some(tx) = holder.lock().expect("lock").take() {
                let _ = tx.send(());
            }
            Ok(())
        })
    }
}

fn failure_cb(
    holder: Arc<Mutex<Option<oneshot::Sender<()>>>>,
) -> impl Fn(&DecodeError) + Send + Sync + 'static {
    move |_| {
        if let Some(tx) = holder.lock().expect("lock").take() {
            let _ = tx.send(());
        }
    }
}

#[rstest]
#[tokio::test]
async fn callbacks_dropped_when_overriding_preamble(
    factory: impl Fn() -> WireframeApp + Send + Sync + Clone + 'static,
) {
    let (hotline_success, hotline_success_rx) = channel_holder();
    let (hotline_failure, hotline_failure_rx) = channel_holder();
    let (other_success, other_success_rx) = channel_holder();
    let (other_failure, other_failure_rx) = channel_holder();

    let server = WireframeServer::new(factory.clone())
        .with_preamble::<HotlinePreamble>()
        .on_preamble_decode_success(success_cb::<HotlinePreamble>(hotline_success.clone()))
        .on_preamble_decode_failure(failure_cb(hotline_failure.clone()))
        .with_preamble::<OtherPreamble>()
        .on_preamble_decode_success(success_cb::<OtherPreamble>(other_success.clone()))
        .on_preamble_decode_failure(failure_cb(other_failure.clone()));

    with_running_server(server, |addr| async move {
        let mut stream = TcpStream::connect(addr).await.expect("connect failed");
        let config = bincode::config::standard()
            .with_big_endian()
            .with_fixed_int_encoding();
        let mut bytes = bincode::encode_to_vec(OtherPreamble(1), config).expect("encode preamble");
        bytes.resize(8, 0);
        stream.write_all(&bytes).await.expect("write failed");
        stream.shutdown().await.expect("shutdown failed");
        // Wait for the success callback before shutting down the server.
        timeout(Duration::from_secs(1), other_success_rx)
            .await
            .expect("timeout waiting for other success")
            .expect("other success send");
    })
    .await;
    assert!(
        timeout(Duration::from_millis(500), other_failure_rx)
            .await
            .is_err(),
        "other failure callback invoked",
    );
    assert!(
        timeout(Duration::from_millis(500), hotline_success_rx)
            .await
            .is_err(),
        "hotline success callback invoked",
    );
    assert!(
        timeout(Duration::from_millis(500), hotline_failure_rx)
            .await
            .is_err(),
        "hotline failure callback invoked",
    );
}
