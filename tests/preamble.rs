use bincode::error::DecodeError;
use tokio::io::{AsyncWriteExt, duplex};
use tokio::net::TcpStream;
use tokio::sync::oneshot;
use tokio::time::{Duration, timeout};
use wireframe::preamble::read_preamble;
use wireframe::{app::WireframeApp, server::WireframeServer};

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

#[tokio::test]
async fn parse_valid_preamble() {
    let (mut client, mut server) = duplex(64);
    let bytes = b"TRTPHOTL\x00\x01\x00\x02";
    client.write_all(bytes).await.unwrap();
    client.shutdown().await.unwrap();
    let (p, _) = read_preamble::<_, HotlinePreamble>(&mut server)
        .await
        .expect("valid preamble");
    eprintln!("decoded: {p:?}");
    p.validate().unwrap();
    assert_eq!(p.magic, HotlinePreamble::MAGIC);
    assert_eq!(p.min_version, 1);
    assert_eq!(p.client_version, 2);
}

#[tokio::test]
async fn invalid_magic_is_error() {
    let (mut client, mut server) = duplex(64);
    let bytes = b"WRONGMAG\x00\x01\x00\x02";
    client.write_all(bytes).await.unwrap();
    client.shutdown().await.unwrap();
    let (preamble, _) = read_preamble::<_, HotlinePreamble>(&mut server)
        .await
        .expect("decoded");
    assert!(preamble.validate().is_err());
}

#[tokio::test]
async fn server_triggers_success_callback() {
    let factory = || WireframeApp::new().expect("WireframeApp::new failed");
    let (success_tx, success_rx) = tokio::sync::oneshot::channel::<HotlinePreamble>();
    let (failure_tx, failure_rx) = tokio::sync::oneshot::channel::<()>();
    let success_tx = std::sync::Arc::new(std::sync::Mutex::new(Some(success_tx)));
    let failure_tx = std::sync::Arc::new(std::sync::Mutex::new(Some(failure_tx)));
    let server = WireframeServer::new(factory)
        .workers(1)
        .with_preamble::<HotlinePreamble>()
        .on_preamble_decode_success({
            let success_tx = success_tx.clone();
            move |p| {
                if let Some(tx) = success_tx.lock().unwrap().take() {
                    let _ = tx.send(p.clone());
                }
            }
        })
        .on_preamble_decode_failure({
            let failure_tx = failure_tx.clone();
            move |_| {
                if let Some(tx) = failure_tx.lock().unwrap().take() {
                    let _ = tx.send(());
                }
            }
        });
    let server = server.bind("127.0.0.1:0".parse().unwrap()).expect("bind");
    let addr = server.local_addr().expect("addr");
    let (shutdown_tx, shutdown_rx) = oneshot::channel::<()>();
    let handle = tokio::spawn(async move {
        server
            .run_with_shutdown(async {
                let _ = shutdown_rx.await;
            })
            .await
            .unwrap();
    });

    let mut stream = TcpStream::connect(addr).await.unwrap();
    let bytes = b"TRTPHOTL\x00\x01\x00\x02";
    stream.write_all(bytes).await.unwrap();
    stream.shutdown().await.unwrap();

    let preamble = timeout(Duration::from_secs(1), success_rx)
        .await
        .expect("timeout waiting for success")
        .expect("success send");
    assert_eq!(preamble.magic, HotlinePreamble::MAGIC);
    assert!(
        timeout(Duration::from_millis(100), failure_rx)
            .await
            .is_err()
    );

    let _ = shutdown_tx.send(());
    handle.await.unwrap();
}

#[tokio::test]
async fn server_triggers_failure_callback() {
    let factory = || WireframeApp::new().expect("WireframeApp::new failed");
    let (success_tx, success_rx) = tokio::sync::oneshot::channel::<HotlinePreamble>();
    let (failure_tx, failure_rx) = tokio::sync::oneshot::channel::<()>();
    let success_tx = std::sync::Arc::new(std::sync::Mutex::new(Some(success_tx)));
    let failure_tx = std::sync::Arc::new(std::sync::Mutex::new(Some(failure_tx)));
    let server = WireframeServer::new(factory)
        .workers(1)
        .with_preamble::<HotlinePreamble>()
        .on_preamble_decode_success({
            let success_tx = success_tx.clone();
            move |p| {
                if let Some(tx) = success_tx.lock().unwrap().take() {
                    let _ = tx.send(p.clone());
                }
            }
        })
        .on_preamble_decode_failure({
            let failure_tx = failure_tx.clone();
            move |_| {
                if let Some(tx) = failure_tx.lock().unwrap().take() {
                    let _ = tx.send(());
                }
            }
        });
    let server = server.bind("127.0.0.1:0".parse().unwrap()).expect("bind");
    let addr = server.local_addr().expect("addr");
    let (shutdown_tx, shutdown_rx) = oneshot::channel::<()>();
    let handle = tokio::spawn(async move {
        server
            .run_with_shutdown(async {
                let _ = shutdown_rx.await;
            })
            .await
            .unwrap();
    });

    let mut stream = TcpStream::connect(addr).await.unwrap();
    let bytes = b"TRTPHOT"; // truncated
    stream.write_all(bytes).await.unwrap();
    stream.shutdown().await.unwrap();

    timeout(Duration::from_secs(1), failure_rx)
        .await
        .expect("timeout waiting for failure")
        .expect("failure send");
    assert!(
        timeout(Duration::from_millis(100), success_rx)
            .await
            .is_err()
    );

    let _ = shutdown_tx.send(());
    handle.await.unwrap();
}
