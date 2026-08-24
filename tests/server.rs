//! Tests for [`WireframeServer`] configuration.
#![cfg(not(loom))]

use std::{io::ErrorKind, net::SocketAddr};

use rstest::rstest;
use tokio::{
    net::TcpStream,
    sync::oneshot,
    time::{Duration, sleep, timeout},
};
use wireframe::server::WireframeServer;
use wireframe_testing::{TestResult, factory, unused_listener};

async fn wait_for_listener_release(addr: SocketAddr) -> TestResult {
    let release_result: TestResult = timeout(Duration::from_secs(1), async {
        loop {
            match TcpStream::connect(addr).await {
                Ok(stream) => drop(stream),
                Err(error) if error.kind() == ErrorKind::ConnectionRefused => return Ok(()),
                Err(error) => return Err(error.into()),
            }
            sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .map_err(|_| format!("server listener at {addr} remained bound for one second"))?;
    release_result?;
    Ok(())
}

#[test]
fn default_worker_count_matches_cpu_count() -> TestResult {
    let server = WireframeServer::new(factory());
    let expected = std::thread::available_parallelism().map_or(1, std::num::NonZeroUsize::get);
    if server.worker_count() != expected {
        return Err(format!(
            "worker count mismatch: actual={}, expected={}",
            server.worker_count(),
            expected
        )
        .into());
    }
    Ok(())
}

#[test]
fn default_workers_at_least_one() -> TestResult {
    let server = WireframeServer::new(factory());
    if server.worker_count() < 1 {
        return Err(format!("worker count below 1: {}", server.worker_count()).into());
    }
    Ok(())
}

#[rstest]
#[case::clamps_zero_to_one(0, 1)]
#[case::retains_large_worker_count(128, 128)]
fn workers_normalizes_configured_values(
    #[case] requested: usize,
    #[case] expected: usize,
) -> TestResult {
    let server = WireframeServer::new(factory()).workers(requested);
    let actual = server.worker_count();
    if actual != expected {
        return Err(format!(
            "worker count mismatch: requested={requested}, actual={actual}, expected={expected}"
        )
        .into());
    }
    Ok(())
}

/// Ensure dropping the readiness receiver logs a warning and does not
/// prevent the server from accepting connections.
#[tokio::test]
async fn readiness_receiver_dropped() -> TestResult {
    let listener = unused_listener()?;
    let server = WireframeServer::new(factory())
        .workers(1)
        .bind_existing_listener(listener)
        .expect("failed to bind existing listener");

    let addr = server.local_addr().expect("local addr missing");
    // Create channel and immediately drop receiver to force send failure
    let (tx_ready, rx_ready) = oneshot::channel();
    drop(rx_ready);

    tokio::spawn(async move {
        server
            .ready_signal(tx_ready)
            .run_with_shutdown(tokio::time::sleep(Duration::from_millis(200)))
            .await
            .expect("server run failed");
    });

    // Wait briefly to ensure server attempted to send readiness signal
    sleep(Duration::from_millis(100)).await;

    // Server should still accept connections
    let _stream = TcpStream::connect(addr).await.expect("connect failed");
    Ok(())
}

#[tokio::test]
async fn aborting_server_supervisor_releases_listener() -> TestResult {
    let listener = unused_listener()?;
    let server = WireframeServer::new(factory())
        .workers(1)
        .bind_existing_listener(listener)
        .expect("failed to bind existing listener");
    let addr = server.local_addr().expect("local addr missing");
    let (ready_tx, ready_rx) = oneshot::channel();
    let handle = tokio::spawn(async move {
        server
            .ready_signal(ready_tx)
            .run_with_shutdown(std::future::pending())
            .await
    });

    timeout(Duration::from_secs(1), ready_rx)
        .await
        .expect("server did not reach readiness")
        .expect("server dropped readiness sender");

    handle.abort();
    let error = handle
        .await
        .expect_err("aborted server supervisor unexpectedly completed");
    if !error.is_cancelled() {
        return Err("server supervisor was not cancelled".into());
    }

    wait_for_listener_release(addr).await
}

#[tokio::test]
async fn dropping_server_supervisor_releases_listener() -> TestResult {
    let listener = unused_listener()?;
    let server = WireframeServer::new(factory())
        .workers(1)
        .bind_existing_listener(listener)
        .expect("failed to bind existing listener");
    let addr = server.local_addr().expect("local addr missing");
    let (ready_tx, mut ready_rx) = oneshot::channel();
    let mut supervisor = Box::pin(
        server
            .ready_signal(ready_tx)
            .run_with_shutdown(std::future::pending()),
    );

    #[expect(
        clippy::integer_division_remainder_used,
        reason = "tokio::select! expands to modulus internally"
    )]
    let readiness: TestResult = timeout(Duration::from_secs(1), async {
        tokio::select! {
            readiness = &mut ready_rx => {
                readiness.map_err(|error| {
                    format!("server dropped readiness sender: {error}").into()
                })
            }
            result = &mut supervisor => {
                Err(format!("server supervisor completed before readiness: {result:?}").into())
            }
        }
    })
    .await
    .map_err(|_| "server did not reach readiness")?;
    readiness?;

    drop(supervisor);
    sleep(Duration::from_millis(10)).await;
    wait_for_listener_release(addr).await
}

#[tokio::test]
async fn server_reaches_readiness_and_accepts_before_shutdown() -> TestResult {
    let listener = unused_listener()?;
    let server = WireframeServer::new(factory())
        .workers(1)
        .bind_existing_listener(listener)
        .expect("failed to bind existing listener");
    let addr = server.local_addr().expect("local addr missing");
    let (ready_tx, ready_rx) = oneshot::channel();
    let (shutdown_tx, shutdown_rx) = oneshot::channel();
    let handle = tokio::spawn(async move {
        server
            .ready_signal(ready_tx)
            .run_with_shutdown(async {
                let _ = shutdown_rx.await;
            })
            .await
    });

    timeout(Duration::from_secs(1), ready_rx)
        .await
        .expect("server did not reach readiness")
        .expect("server dropped readiness sender");

    let stream = TcpStream::connect(addr)
        .await
        .expect("ready server did not accept connections");
    drop(stream);
    shutdown_tx
        .send(())
        .expect("server shutdown receiver was dropped");
    handle.await??;
    Ok(())
}
