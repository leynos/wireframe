#![cfg(not(loom))]
#![expect(
    clippy::excessive_nesting,
    reason = "async closures within builder patterns are inherently nested"
)]
//! Integration tests for client connection lifecycle callbacks.
//!
//! These tests verify that the client lifecycle hooks (setup, teardown, error)
//! work correctly when the client interacts with a real server.

use std::sync::{
    Arc,
    atomic::{AtomicUsize, Ordering},
};

use tokio::net::TcpListener;
use wireframe::WireframeClient;

mod common;
use common::TestResult;

/// Test that setup and teardown callbacks are both invoked for a full
/// connection lifecycle.
#[tokio::test]
#[expect(
    clippy::panic_in_result_fn,
    reason = "asserts provide clearer diagnostics in tests"
)]
async fn client_setup_and_teardown_callbacks_run() -> TestResult<()> {
    let setup_count = Arc::new(AtomicUsize::new(0));
    let teardown_count = Arc::new(AtomicUsize::new(0));
    let setup = setup_count.clone();
    let teardown = teardown_count.clone();

    let listener = TcpListener::bind("127.0.0.1:0").await?;
    let addr = listener.local_addr()?;

    // Server accepts and then closes
    let server = tokio::spawn(async move {
        let (stream, _) = listener.accept().await.expect("accept");
        drop(stream); // Close immediately
    });

    let client = WireframeClient::builder()
        .on_connection_setup(move || {
            let setup = setup.clone();
            async move {
                setup.fetch_add(1, Ordering::SeqCst);
                42u32
            }
        })
        .on_connection_teardown(move |state: u32| {
            let teardown = teardown.clone();
            async move {
                assert_eq!(state, 42, "teardown should receive state from setup");
                teardown.fetch_add(1, Ordering::SeqCst);
            }
        })
        .connect(addr)
        .await?;

    assert_eq!(
        setup_count.load(Ordering::SeqCst),
        1,
        "setup callback should run exactly once"
    );

    client.close().await;

    assert_eq!(
        teardown_count.load(Ordering::SeqCst),
        1,
        "teardown callback should run exactly once"
    );

    server.await?;
    Ok(())
}

/// Test that the error hook is invoked when the server disconnects unexpectedly.
#[tokio::test]
#[expect(
    clippy::panic_in_result_fn,
    reason = "asserts provide clearer diagnostics in tests"
)]
async fn client_error_hook_invoked_on_disconnect() -> TestResult<()> {
    let error_count = Arc::new(AtomicUsize::new(0));
    let count = error_count.clone();

    let listener = TcpListener::bind("127.0.0.1:0").await?;
    let addr = listener.local_addr()?;

    // Server accepts and then closes immediately
    let server = tokio::spawn(async move {
        let (stream, _) = listener.accept().await.expect("accept");
        drop(stream);
    });

    let mut client = WireframeClient::builder()
        .on_error(move |_err| {
            let count = count.clone();
            async move {
                count.fetch_add(1, Ordering::SeqCst);
            }
        })
        .connect(addr)
        .await?;

    server.await?;

    // Try to receive - should fail and invoke error hook
    let result: Result<Vec<u8>, wireframe::ClientError> = client.receive().await;
    assert!(result.is_err(), "receive should fail after disconnect");

    assert_eq!(
        error_count.load(Ordering::SeqCst),
        1,
        "error callback should be invoked on disconnect"
    );

    Ok(())
}

#[derive(bincode::Encode, bincode::BorrowDecode)]
struct ClientHello {
    version: u16,
}

#[derive(bincode::Encode, bincode::BorrowDecode)]
struct ServerAck {
    accepted: bool,
}

/// Test that lifecycle hooks can be combined with preamble callbacks.
#[tokio::test]
#[expect(
    clippy::panic_in_result_fn,
    reason = "asserts provide clearer diagnostics in tests"
)]
async fn client_lifecycle_hooks_work_with_preamble() -> TestResult<()> {
    use futures::FutureExt;
    use wireframe::preamble::{read_preamble, write_preamble};

    let setup_count = Arc::new(AtomicUsize::new(0));
    let teardown_count = Arc::new(AtomicUsize::new(0));
    let preamble_count = Arc::new(AtomicUsize::new(0));
    let setup = setup_count.clone();
    let teardown = teardown_count.clone();
    let preamble = preamble_count.clone();

    let listener = TcpListener::bind("127.0.0.1:0").await?;
    let addr = listener.local_addr()?;

    // Server reads preamble and sends ack
    let server = tokio::spawn(async move {
        let (mut stream, _) = listener.accept().await.expect("accept");
        let (_hello, _) = read_preamble::<_, ClientHello>(&mut stream)
            .await
            .expect("read preamble");
        write_preamble(&mut stream, &ServerAck { accepted: true })
            .await
            .expect("write ack");
        drop(stream);
    });

    let client = WireframeClient::builder()
        .with_preamble(ClientHello { version: 1 })
        .on_preamble_success(move |_preamble, stream| {
            let preamble = preamble.clone();
            async move {
                preamble.fetch_add(1, Ordering::SeqCst);
                let (ack, leftover) = read_preamble::<_, ServerAck>(stream).await.map_err(|e| {
                    std::io::Error::new(std::io::ErrorKind::InvalidData, e.to_string())
                })?;
                assert!(ack.accepted, "server should accept preamble");
                Ok(leftover)
            }
            .boxed()
        })
        .on_connection_setup(move || {
            let setup = setup.clone();
            async move {
                setup.fetch_add(1, Ordering::SeqCst);
                "session-state"
            }
        })
        .on_connection_teardown(move |_: &str| {
            let teardown = teardown.clone();
            async move {
                teardown.fetch_add(1, Ordering::SeqCst);
            }
        })
        .connect(addr)
        .await?;

    server.await?;

    assert_eq!(
        preamble_count.load(Ordering::SeqCst),
        1,
        "preamble callback should run"
    );
    assert_eq!(
        setup_count.load(Ordering::SeqCst),
        1,
        "setup callback should run"
    );

    client.close().await;

    assert_eq!(
        teardown_count.load(Ordering::SeqCst),
        1,
        "teardown callback should run"
    );

    Ok(())
}
