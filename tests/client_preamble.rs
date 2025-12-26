//! Integration tests for client preamble exchange.

#![cfg(not(loom))]

use std::{
    io,
    sync::{Arc, Mutex},
    time::Duration,
};

use futures::FutureExt;
use rstest::rstest;
use tokio::{io::AsyncReadExt, net::TcpListener, sync::oneshot, time::timeout};
use wireframe::{
    ClientError,
    client::WireframeClient,
    preamble::{read_preamble, write_preamble},
};

mod common;
use common::TestResult;

/// A simple preamble for testing.
#[derive(Debug, Clone, PartialEq, Eq, bincode::Encode, bincode::BorrowDecode)]
struct TestPreamble {
    magic: [u8; 4],
    version: u16,
}

impl TestPreamble {
    const MAGIC: [u8; 4] = *b"TEST";

    fn new(version: u16) -> Self {
        Self {
            magic: Self::MAGIC,
            version,
        }
    }

    fn is_valid(&self) -> bool { self.magic == Self::MAGIC }
}

/// Server acknowledgement preamble.
#[derive(Debug, Clone, PartialEq, Eq, bincode::Encode, bincode::BorrowDecode)]
struct ServerAck {
    accepted: bool,
}

type Holder<T> = Arc<Mutex<Option<oneshot::Sender<T>>>>;

fn channel_holder<T>() -> (Holder<T>, oneshot::Receiver<T>) {
    let (tx, rx) = oneshot::channel();
    (Arc::new(Mutex::new(Some(tx))), rx)
}

fn take_sender<T>(holder: &Mutex<Option<oneshot::Sender<T>>>) -> Option<oneshot::Sender<T>> {
    holder.lock().ok().and_then(|mut guard| guard.take())
}

#[rstest]
#[tokio::test]
async fn client_sends_preamble_to_server() -> TestResult {
    let listener = TcpListener::bind("127.0.0.1:0").await?;
    let addr = listener.local_addr()?;

    // Server task: accept connection and read preamble.
    let server = tokio::spawn(async move {
        let (mut stream, _) = listener.accept().await.expect("accept");
        let (preamble, _leftover) = read_preamble::<_, TestPreamble>(&mut stream)
            .await
            .expect("read preamble");
        assert!(preamble.is_valid(), "preamble magic should be valid");
        assert_eq!(preamble.version, 42);
        // Send acknowledgement.
        write_preamble(&mut stream, &ServerAck { accepted: true })
            .await
            .expect("write ack");
    });

    // Client: connect with preamble.
    let _client = WireframeClient::builder()
        .with_preamble(TestPreamble::new(42))
        .on_preamble_success(|_preamble, stream| {
            async move {
                // Read server acknowledgement.
                let (ack, leftover) = read_preamble::<_, ServerAck>(stream)
                    .await
                    .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e.to_string()))?;
                assert!(ack.accepted, "server should accept preamble");
                Ok(leftover)
            }
            .boxed()
        })
        .connect(addr)
        .await?;

    server.await?;
    Ok(())
}

#[rstest]
#[tokio::test]
async fn client_success_callback_receives_preamble() -> TestResult {
    let listener = TcpListener::bind("127.0.0.1:0").await?;
    let addr = listener.local_addr()?;
    let (preamble_holder, preamble_rx) = channel_holder::<TestPreamble>();

    // Server task: accept and read preamble, then close.
    let server = tokio::spawn(async move {
        let (mut stream, _) = listener.accept().await.expect("accept");
        let (_preamble, _) = read_preamble::<_, TestPreamble>(&mut stream)
            .await
            .expect("read preamble");
        drop(stream);
    });

    let _client = WireframeClient::builder()
        .with_preamble(TestPreamble::new(123))
        .on_preamble_success({
            let holder = preamble_holder.clone();
            move |preamble, _stream| {
                let holder = holder.clone();
                let clone = preamble.clone();
                async move {
                    if let Some(tx) = take_sender(&holder) {
                        let _ = tx.send(clone);
                    }
                    Ok(Vec::new())
                }
                .boxed()
            }
        })
        .connect(addr)
        .await?;

    let received = timeout(Duration::from_secs(1), preamble_rx).await??;
    assert_eq!(received.version, 123);

    server.await?;
    Ok(())
}

#[rstest]
#[tokio::test]
async fn client_preamble_timeout_triggers_failure() -> TestResult {
    let listener = TcpListener::bind("127.0.0.1:0").await?;
    let addr = listener.local_addr()?;
    let (failure_holder, failure_rx) = channel_holder::<()>();

    // Server task: accept but don't read (simulating slow server).
    let server = tokio::spawn(async move {
        let (_stream, _) = listener.accept().await.expect("accept");
        // Hold the connection open but don't respond.
        tokio::time::sleep(Duration::from_secs(5)).await;
    });

    let result = WireframeClient::builder()
        .with_preamble(TestPreamble::new(1))
        .preamble_timeout(Duration::from_millis(50))
        .on_preamble_success(|_preamble, stream| {
            async move {
                // Try to read server response - this should timeout.
                let mut buf = [0u8; 1];
                stream.read_exact(&mut buf).await?;
                Ok(Vec::new())
            }
            .boxed()
        })
        .on_preamble_failure({
            let holder = failure_holder.clone();
            move |err, _stream| {
                let holder = holder.clone();
                async move {
                    assert!(
                        matches!(err, ClientError::PreambleTimeout),
                        "expected PreambleTimeout, got {err:?}",
                    );
                    if let Some(tx) = take_sender(&holder) {
                        let _ = tx.send(());
                    }
                    Ok(())
                }
                .boxed()
            }
        })
        .connect(addr)
        .await;

    assert!(
        matches!(result, Err(ClientError::PreambleTimeout)),
        "expected PreambleTimeout error"
    );
    timeout(Duration::from_secs(1), failure_rx).await??;

    server.abort();
    Ok(())
}

#[rstest]
#[tokio::test]
async fn client_without_preamble_connects_normally() -> TestResult {
    let listener = TcpListener::bind("127.0.0.1:0").await?;
    let addr = listener.local_addr()?;

    let server = tokio::spawn(async move {
        let (_stream, _) = listener.accept().await.expect("accept");
        // Just accept and hold connection briefly.
        tokio::time::sleep(Duration::from_millis(100)).await;
    });

    // Connect without preamble.
    let _client = WireframeClient::builder().connect(addr).await?;

    server.await?;
    Ok(())
}

#[rstest]
#[tokio::test]
async fn client_preamble_write_only_no_response() -> TestResult {
    let listener = TcpListener::bind("127.0.0.1:0").await?;
    let addr = listener.local_addr()?;
    let (success_holder, success_rx) = channel_holder::<()>();

    // Server task: accept and read preamble.
    let server = tokio::spawn(async move {
        let (mut stream, _) = listener.accept().await.expect("accept");
        let (preamble, _) = read_preamble::<_, TestPreamble>(&mut stream)
            .await
            .expect("read preamble");
        assert!(preamble.is_valid());
        // Don't send any response - just hold connection.
        tokio::time::sleep(Duration::from_millis(100)).await;
    });

    // Client: connect with preamble but no success callback (no response expected).
    let _client = WireframeClient::builder()
        .with_preamble(TestPreamble::new(99))
        .on_preamble_success({
            let holder = success_holder.clone();
            move |_preamble, _stream| {
                let holder = holder.clone();
                async move {
                    // Don't read anything, just signal success.
                    if let Some(tx) = take_sender(&holder) {
                        let _ = tx.send(());
                    }
                    Ok(Vec::new())
                }
                .boxed()
            }
        })
        .connect(addr)
        .await?;

    timeout(Duration::from_secs(1), success_rx).await??;
    server.await?;
    Ok(())
}
