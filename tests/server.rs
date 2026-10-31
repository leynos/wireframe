//! Tests for [`WireframeServer`] configuration.
#![cfg(not(loom))]

use std::io::{ErrorKind, Result as IoResult};

use rstest::rstest;
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::TcpStream,
    sync::oneshot,
    time::{Duration, sleep, timeout},
};
use tracing::Instrument;
use tracing_test::traced_test;
use wireframe::server::WireframeServer;
use wireframe_testing::{
    TestApp,
    TestResult,
    factory,
    unused_listener,
    wait_for_listener_release,
    wait_for_server_readiness,
};

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

    wait_for_server_readiness(ready_rx).await?;

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

    wait_for_server_readiness(ready_rx).await?;

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

/// Ensure the server survives panicking connection tasks.
///
/// `#[traced_test]` precedes `#[rstest]` so the injected `logs_assert` helper
/// stays in scope, matching the parameterised unit tests in
/// `src/server/connection_spawner_tests.rs`.
#[traced_test]
#[rstest]
#[tokio::test]
async fn connection_panic_is_caught(
    factory: impl Fn() -> TestApp + Send + Sync + Clone + 'static,
) -> TestResult {
    let listener = unused_listener()?;
    let server = WireframeServer::new(move || {
        factory()
            .on_connection_setup(|| async { panic!("boom") })
            .expect("failed to install panic setup callback")
    })
    .workers(1)
    .bind_existing_listener(listener)?;
    let addr = server
        .local_addr()
        .ok_or_else(|| "local address missing".to_string())?;

    let (ready_tx, ready_rx) = oneshot::channel();
    let (shutdown_tx, shutdown_rx) = oneshot::channel();
    let span = tracing::Span::current();
    let handle = tokio::spawn(
        async move {
            server
                .ready_signal(ready_tx)
                .run_with_shutdown(async {
                    let _ = shutdown_rx.await;
                })
                .await
        }
        .instrument(span),
    );

    wait_for_server_readiness(ready_rx).await?;

    let mut first = TcpStream::connect(addr)
        .await
        .expect("first connection should succeed");
    let peer_addr = first.local_addr().expect("first connection peer address");
    drive_until_closed(&mut first, "first connection").await?;
    drop(first);

    // Opening the second connection only once the first has closed proves the
    // server accepts connections *after* a handler panicked, rather than
    // racing both connections into their handlers.
    let mut second = TcpStream::connect(addr)
        .await
        .expect("second connection should succeed after panic");
    let second_peer_addr = second.local_addr().expect("second connection peer address");
    drive_until_closed(&mut second, "second connection").await?;
    drop(second);

    shutdown_tx
        .send(())
        .expect("server shutdown receiver was dropped");
    handle.await.expect("server join error")?;
    tokio::task::yield_now().await;

    logs_assert(|lines: &[&str]| {
        for peer in [peer_addr, second_peer_addr] {
            let logged = lines.iter().any(|line| {
                line.contains("connection task panicked")
                    && line.contains("panic=boom")
                    && line.contains(&format!("peer_addr=Some({peer})"))
            });
            if !logged {
                return Err(format!("panic log not found for peer {peer}"));
            }
        }
        Ok(())
    });
    Ok(())
}

/// Send data on a client stream, then wait until the server closes it.
///
/// Closing proves the connection reached a handler, because the panicking task
/// owns and drops the accepted socket. The panicking handler may close the
/// connection before the write lands, so a closed-peer write error counts as
/// handled rather than as a failure. A graceful close yields end-of-file; a
/// reset counts too, because the handler never drains the bytes sent, so
/// dropping its socket rejects the leftover data with an RST.
async fn drive_until_closed(stream: &mut TcpStream, label: &str) -> TestResult {
    if let Err(error) = stream.write_all(&[0; 8]).await
        && !is_peer_closed_error(&error)
    {
        return Err(format!("{label} write failed: {error}").into());
    }

    let mut probe = [0_u8; 1];
    match timeout(Duration::from_secs(5), stream.read(&mut probe)).await {
        Ok(result) if peer_closed(&result) => Ok(()),
        Ok(Ok(read)) => Err(format!("{label} returned {read} bytes instead of closing").into()),
        Ok(Err(error)) => Err(format!("{label} read failed: {error}").into()),
        Err(_) => Err(format!("{label} was not handled").into()),
    }
}

/// Report whether a read observed the peer closing the connection.
fn peer_closed(result: &IoResult<usize>) -> bool {
    match result {
        Ok(0) => true,
        Err(error) => is_peer_closed_error(error),
        Ok(_) => false,
    }
}

/// Report whether an I/O error means the peer had already closed.
fn is_peer_closed_error(error: &std::io::Error) -> bool {
    matches!(
        error.kind(),
        ErrorKind::BrokenPipe | ErrorKind::ConnectionReset
    )
}
