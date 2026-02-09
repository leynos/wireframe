//! Tests for [`WireframeServer`] configuration.
#![cfg(not(loom))]

use wireframe::server::WireframeServer;
use wireframe_testing::{TestResult, factory, unused_listener};

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

#[test]
fn workers_method_enforces_minimum() -> TestResult {
    let server = WireframeServer::new(factory()).workers(0);
    if server.worker_count() != 1 {
        return Err(format!(
            "worker count should clamp to 1, got {}",
            server.worker_count()
        )
        .into());
    }
    Ok(())
}

#[test]
fn workers_accepts_large_values() -> TestResult {
    let server = WireframeServer::new(factory()).workers(128);
    if server.worker_count() != 128 {
        return Err(format!(
            "worker count should be 128 after config, got {}",
            server.worker_count()
        )
        .into());
    }
    Ok(())
}

/// Ensure dropping the readiness receiver logs a warning and does not
/// prevent the server from accepting connections.
#[tokio::test]
async fn readiness_receiver_dropped() -> TestResult {
    use tokio::{
        net::TcpStream,
        sync::oneshot,
        time::{Duration, sleep},
    };

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
