//! Integration tests for awaitable `WireframeServer` shutdown.
#![cfg(not(loom))]

use std::{io, net::SocketAddr, sync::Arc};

use futures::future::join_all;
use rstest::{fixture, rstest};
use tokio::{
    net::TcpStream,
    sync::{Notify, oneshot},
    time::{Duration, Instant, timeout},
};
use wireframe::{
    preamble::write_preamble,
    server::{ServerShutdown, WireframeServer},
};
use wireframe_testing::{
    TestApp,
    TestResult,
    factory,
    unused_listener,
    wait_for_listener_release,
    wait_for_server_readiness,
};

#[rstest]
#[tokio::test]
async fn stop_then_drained_releases_listener_without_polling(
    #[future]
    #[with(1)]
    spawn_ready_server: TestResult<(SocketAddr, ServerShutdown)>,
) -> TestResult {
    let (addr, shutdown) = spawn_ready_server.await?;
    shutdown.stop();
    shutdown.drained().await?;
    assert_connection_refused(addr).await
}

#[rstest]
#[tokio::test]
async fn dropping_final_shutdown_handle_requests_shutdown(
    #[future]
    #[with(1)]
    spawn_ready_server: TestResult<(SocketAddr, ServerShutdown)>,
) -> TestResult {
    let (addr, shutdown) = spawn_ready_server.await?;
    drop(shutdown);
    wait_for_listener_release(addr).await?;
    assert_connection_refused(addr).await
}

#[rstest]
#[tokio::test]
async fn stop_preserves_graceful_drain_for_in_flight_connections(
    #[future] spawn_ready_server_with_connection: TestResult<(
        SocketAddr,
        ServerShutdown,
        TcpStream,
    )>,
) -> TestResult {
    let (_addr, shutdown, stream) = spawn_ready_server_with_connection.await?;
    shutdown.stop();
    let mut drain = Box::pin(shutdown.drained());
    if timeout(Duration::from_millis(50), &mut drain).await.is_ok() {
        return Err("server drained before the in-flight connection closed".into());
    }

    drop(stream);
    timeout(Duration::from_secs(1), drain)
        .await
        .map_err(|_| "server did not drain after connection closed")??;
    Ok(())
}

#[rstest]
#[tokio::test]
async fn concurrent_stops_and_drains_converge_to_one_clean_outcome(
    #[future]
    #[with(2)]
    spawn_ready_server: TestResult<(SocketAddr, ServerShutdown)>,
) -> TestResult {
    let (addr, shutdown) = spawn_ready_server.await?;
    let drains = join_all((0..8).map(|_| {
        let shutdown = shutdown.clone();
        async move {
            shutdown.stop();
            shutdown.drained().await
        }
    }))
    .await;
    for result in drains {
        result?;
    }
    assert_connection_refused(addr).await
}

#[rstest]
#[tokio::test]
async fn stop_is_non_blocking_while_connections_keep_the_server_busy(
    #[future] spawn_ready_server_with_connection: TestResult<(
        SocketAddr,
        ServerShutdown,
        TcpStream,
    )>,
) -> TestResult {
    let (addr, shutdown, first_stream) = spawn_ready_server_with_connection.await?;
    let additional_streams = join_all((0..16).map(|_| TcpStream::connect(addr))).await;
    let mut streams = vec![first_stream];
    for stream in additional_streams {
        streams.push(stream?);
    }

    let started = Instant::now();
    shutdown.stop();
    if started.elapsed() >= Duration::from_secs(1) {
        return Err("stop waited for active connections".into());
    }
    drop(streams);
    timeout(Duration::from_secs(1), shutdown.drained())
        .await
        .map_err(|_| "server did not drain after busy connections closed")??;
    Ok(())
}

#[fixture]
async fn spawn_ready_server(
    #[default(1)] workers: usize,
) -> TestResult<(SocketAddr, ServerShutdown)> {
    let listener = unused_listener()?;
    let server = WireframeServer::new(factory())
        .workers(workers)
        .bind_existing_listener(listener)?;
    let addr = server.local_addr().ok_or("server local address missing")?;
    let (ready_tx, ready_rx) = oneshot::channel();
    let shutdown = server.ready_signal(ready_tx).spawn().await?;

    wait_for_server_readiness(ready_rx).await?;
    Ok((addr, shutdown))
}

#[fixture]
async fn spawn_ready_server_with_connection() -> TestResult<(SocketAddr, ServerShutdown, TcpStream)>
{
    let setup = Arc::new(Notify::new());
    let listener = unused_listener()?;
    let server = WireframeServer::from_app(setup_notifying_app(Arc::clone(&setup))?)
        .workers(1)
        .with_preamble::<ShutdownTestPreamble>()
        .bind_existing_listener(listener)?;
    let addr = server.local_addr().ok_or("server local address missing")?;
    let (ready_tx, ready_rx) = oneshot::channel();
    let shutdown = server.ready_signal(ready_tx).spawn().await?;

    wait_for_server_readiness(ready_rx).await?;
    let setup_complete = setup.notified();
    let mut stream = TcpStream::connect(addr).await?;
    write_preamble(&mut stream, &ShutdownTestPreamble(1)).await?;
    timeout(Duration::from_secs(1), setup_complete)
        .await
        .map_err(|_| "connection setup did not complete")?;
    Ok((addr, shutdown, stream))
}

fn setup_notifying_app(setup: Arc<Notify>) -> TestResult<TestApp> {
    TestApp::default()
        .on_connection_setup(move || {
            let setup = Arc::clone(&setup);
            async move { setup.notify_one() }
        })
        .map_err(Into::into)
}

#[derive(bincode::Encode, bincode::BorrowDecode)]
struct ShutdownTestPreamble(u8);

async fn assert_connection_refused(addr: std::net::SocketAddr) -> TestResult {
    match TcpStream::connect(addr).await {
        Err(error) if error.kind() == io::ErrorKind::ConnectionRefused => Ok(()),
        Ok(stream) => {
            drop(stream);
            Err(format!("listener at {addr} accepted a connection after drain").into())
        }
        Err(error) => Err(error.into()),
    }
}
