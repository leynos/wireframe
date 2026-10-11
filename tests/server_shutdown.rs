//! Integration tests for awaitable `WireframeServer` shutdown.
#![cfg(not(loom))]

use std::{io, net::SocketAddr, sync::Arc};

use futures::future::join_all;
use proptest::{
    prelude::*,
    test_runner::{TestCaseError, TestCaseResult},
};
use rstest::{fixture, rstest};
use tokio::{
    net::TcpStream,
    runtime::Builder,
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
    wait_for_listener_release(addr).await
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

proptest! {
    #![proptest_config(ProptestConfig {
        cases: 12,
        ..ProptestConfig::default()
    })]

    /// Generated clone, stop, observer, and drop sequences converge on one clean drain.
    #[test]
    fn generated_server_shutdown_operations_converge(
        requested_workers in prop_oneof![Just(0usize), Just(1), 2usize..=4],
        extra_operations in prop::collection::vec(shutdown_operation_strategy(), 0..=8),
    ) {
        run_generated_shutdown_case(requested_workers, extra_operations)?;
    }
}

#[derive(Clone, Copy, Debug)]
enum ShutdownOperation {
    Clone(usize),
    Stop(usize),
    ObserveDrain(usize),
    DropClone(usize),
}

fn shutdown_operation_strategy() -> impl Strategy<Value = ShutdownOperation> {
    prop_oneof![
        (0usize..=9).prop_map(ShutdownOperation::Clone),
        (0usize..=9).prop_map(ShutdownOperation::Stop),
        (0usize..=9).prop_map(ShutdownOperation::ObserveDrain),
        (0usize..=9).prop_map(ShutdownOperation::DropClone),
    ]
}

fn run_generated_shutdown_case(
    requested_workers: usize,
    extra_operations: Vec<ShutdownOperation>,
) -> TestCaseResult {
    let runtime = Builder::new_current_thread()
        .enable_all()
        .build()
        .map_err(|error| TestCaseError::fail(error.to_string()))?;
    runtime.block_on(exercise_generated_shutdown_case(
        requested_workers,
        extra_operations,
    ))
}

async fn exercise_generated_shutdown_case(
    requested_workers: usize,
    extra_operations: Vec<ShutdownOperation>,
) -> TestCaseResult {
    let (addr, root) = spawn_ready_server_for_workers(requested_workers)
        .await
        .map_err(|error| TestCaseError::fail(format!("server setup failed: {error}")))?;
    let mut clones = Vec::new();
    let mut observers = Vec::new();
    let mut operations = Vec::with_capacity(extra_operations.len() + 5);
    operations.push(ShutdownOperation::Clone(0));
    operations.extend(extra_operations);
    operations.extend([
        ShutdownOperation::ObserveDrain(0),
        ShutdownOperation::Clone(0),
        ShutdownOperation::DropClone(0),
        ShutdownOperation::Stop(0),
    ]);

    for operation in operations {
        match operation {
            ShutdownOperation::Clone(source) => {
                clones.push(select_shutdown_handle(&root, &clones, source).clone());
            }
            ShutdownOperation::Stop(target) => {
                select_shutdown_handle(&root, &clones, target).stop();
            }
            ShutdownOperation::ObserveDrain(target) => {
                let shutdown = select_shutdown_handle(&root, &clones, target).clone();
                observers.push(tokio::spawn(async move { shutdown.drained().await }));
                tokio::task::yield_now().await;
            }
            ShutdownOperation::DropClone(target) => {
                if target < clones.len() {
                    drop(clones.remove(target));
                }
            }
        }
    }

    // Keep the root alive through every generated operation and make the
    // terminal request independent of which operations the generator chose.
    root.stop();
    drop(clones);

    timeout(Duration::from_secs(1), async {
        for (index, result) in join_all(observers).await.into_iter().enumerate() {
            let outcome = result.map_err(|error| {
                TestCaseError::fail(format!("drain observer {index} task failed: {error}"))
            })?;
            prop_assert!(
                outcome.is_ok(),
                "drain observer {index} returned an abnormal outcome: {outcome:?}"
            );
        }
        wait_for_listener_release(addr)
            .await
            .map_err(|error| TestCaseError::fail(format!("listener was not released: {error}")))?;
        Ok(())
    })
    .await
    .map_err(|_| TestCaseError::fail("server shutdown did not complete within one second"))??;

    Ok(())
}

fn select_shutdown_handle<'a>(
    root: &'a ServerShutdown,
    clones: &'a [ServerShutdown],
    selector: usize,
) -> &'a ServerShutdown {
    if selector == 0 {
        root
    } else {
        clones.get(selector - 1).unwrap_or(root)
    }
}

#[fixture]
async fn spawn_ready_server(
    #[default(1)] workers: usize,
) -> TestResult<(SocketAddr, ServerShutdown)> {
    spawn_ready_server_for_workers(workers).await
}

async fn spawn_ready_server_for_workers(
    workers: usize,
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
