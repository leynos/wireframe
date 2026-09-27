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
    atomic::{AtomicU32, AtomicUsize, Ordering},
};

#[path = "common/fallible_assertions/check.rs"]
mod fallible_check;
#[path = "common/fallible_assertions/check_eq.rs"]
mod fallible_check_eq;

use fallible_check::check;
use fallible_check_eq::check_eq;
use tokio::net::TcpListener;
use wireframe::{
    client::WireframeClient,
    preamble::{read_preamble, write_preamble},
};
use wireframe_testing::TestResult;

/// Test that setup and teardown callbacks are both invoked for a full
/// connection lifecycle.
#[tokio::test]
async fn client_setup_and_teardown_callbacks_run() -> TestResult<()> {
    let setup_count = Arc::new(AtomicUsize::new(0));
    let teardown_count = Arc::new(AtomicUsize::new(0));
    let teardown_state = Arc::new(AtomicU32::new(0));
    let setup = setup_count.clone();
    let teardown = teardown_count.clone();
    let observed_teardown_state = teardown_state.clone();

    let listener = TcpListener::bind("127.0.0.1:0").await?;
    let addr = listener.local_addr()?;

    // Server accepts and then closes
    let server = tokio::spawn(async move {
        let (stream, _) = listener.accept().await?;
        drop(stream); // Close immediately
        Ok::<(), wireframe::testkit::TestError>(())
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
            let observed_teardown_state = observed_teardown_state.clone();
            async move {
                observed_teardown_state.store(state, Ordering::SeqCst);
                teardown.fetch_add(1, Ordering::SeqCst);
            }
        })
        .connect(addr)
        .await?;

    check_eq(
        setup_count.load(Ordering::SeqCst),
        1,
        "setup callback should run exactly once",
    )?;

    client.close().await;

    check_eq(
        teardown_state.load(Ordering::SeqCst),
        42,
        "teardown should receive state from setup",
    )?;
    check_eq(
        teardown_count.load(Ordering::SeqCst),
        1,
        "teardown callback should run exactly once",
    )?;

    server.await??;
    Ok(())
}

/// Test that the error hook is invoked when the server disconnects unexpectedly.
#[tokio::test]
async fn client_error_hook_invoked_on_disconnect() -> TestResult<()> {
    let error_count = Arc::new(AtomicUsize::new(0));
    let count = error_count.clone();

    let listener = TcpListener::bind("127.0.0.1:0").await?;
    let addr = listener.local_addr()?;

    // Server accepts and then closes immediately
    let server = tokio::spawn(async move {
        let (stream, _) = listener.accept().await?;
        drop(stream);
        Ok::<(), wireframe::testkit::TestError>(())
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

    server.await??;

    // Try to receive - should fail and invoke error hook
    let result: Result<Vec<u8>, wireframe::client::ClientError> = client.receive().await;
    check(
        result.is_err(),
        format!("receive should fail after disconnect, got {result:?}"),
    )?;

    check_eq(
        error_count.load(Ordering::SeqCst),
        1,
        "error callback should be invoked on disconnect",
    )?;

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

async fn serve_preamble_ack(listener: TcpListener) -> TestResult<()> {
    let (mut stream, _) = listener.accept().await?;
    let (_hello, _) = read_preamble::<_, ClientHello>(&mut stream).await?;
    write_preamble(&mut stream, &ServerAck { accepted: true }).await?;
    drop(stream);
    Ok(())
}

/// Test that lifecycle hooks can be combined with preamble callbacks.
#[tokio::test]
async fn client_lifecycle_hooks_work_with_preamble() -> TestResult<()> {
    use futures::FutureExt;

    let setup_count = Arc::new(AtomicUsize::new(0));
    let teardown_count = Arc::new(AtomicUsize::new(0));
    let preamble_count = Arc::new(AtomicUsize::new(0));
    let setup = setup_count.clone();
    let teardown = teardown_count.clone();
    let preamble = preamble_count.clone();

    let listener = TcpListener::bind("127.0.0.1:0").await?;
    let addr = listener.local_addr()?;

    let server = tokio::spawn(serve_preamble_ack(listener));

    let client = WireframeClient::builder()
        .with_preamble(ClientHello { version: 1 })
        .on_preamble_success(move |_preamble, stream| {
            let preamble = preamble.clone();
            async move {
                preamble.fetch_add(1, Ordering::SeqCst);
                let (ack, leftover) = read_preamble::<_, ServerAck>(stream).await.map_err(|e| {
                    std::io::Error::new(std::io::ErrorKind::InvalidData, e.to_string())
                })?;
                if !ack.accepted {
                    return Err(std::io::Error::new(
                        std::io::ErrorKind::InvalidData,
                        "server should accept preamble",
                    ));
                }
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

    server.await??;

    check_eq(
        preamble_count.load(Ordering::SeqCst),
        1,
        "preamble callback should run",
    )?;
    check_eq(
        setup_count.load(Ordering::SeqCst),
        1,
        "setup callback should run",
    )?;

    client.close().await;

    check_eq(
        teardown_count.load(Ordering::SeqCst),
        1,
        "teardown callback should run",
    )?;

    Ok(())
}
