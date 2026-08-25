//! Bounded lifecycle-model tests for server-supervisor cancellation.
//!
//! Each model run exercises every configured worker-count and terminal-action
//! combination against a current-thread Tokio runtime. The model keeps the
//! runtime alive while dropped and aborted supervisors release their listeners.
#![cfg(not(loom))]

use std::future;

use proptest::{
    prelude::*,
    test_runner::{TestCaseError, TestCaseResult},
};
use tokio::{
    net::TcpStream,
    runtime::Builder,
    sync::oneshot,
    time::{Duration, timeout},
};
use wireframe::server::WireframeServer;
use wireframe_testing::{
    TestResult,
    factory,
    unused_listener,
    wait_for_listener_release,
    wait_for_server_readiness,
};

const WORKER_COUNTS: [usize; 4] = [0, 1, 2, 4];

#[derive(Clone, Copy, Debug)]
enum TerminalAction {
    Graceful,
    Drop,
    Abort,
}

impl TerminalAction {
    const ALL: [Self; 3] = [Self::Graceful, Self::Drop, Self::Abort];
}

fn terminal_actions() -> impl Strategy<Value = TerminalAction> {
    prop_oneof![
        Just(TerminalAction::Graceful),
        Just(TerminalAction::Drop),
        Just(TerminalAction::Abort),
    ]
}

/// Exercise the bounded supervisor model for every worker/action combination.
///
/// The worker-count domain contains normalization (`0`), one worker, and two
/// multi-worker configurations. Each action starts from a ready server, while
/// the graceful path also accepts a connection, so immediate guard
/// cancellation is detected before the terminal transition is applied.
#[tokio::test(flavor = "current_thread")]
async fn bounded_server_supervisor_lifecycle_model() -> TestResult {
    for requested_workers in WORKER_COUNTS {
        for action in TerminalAction::ALL {
            exercise_lifecycle(requested_workers, action).await?;
        }
    }
    Ok(())
}

proptest! {
    #![proptest_config(ProptestConfig {
        cases: 12,
        .. ProptestConfig::default()
    })]

    /// Generate bounded lifecycle cases in addition to the exhaustive matrix.
    #[test]
    fn generated_server_supervisor_lifecycle_cases(
        requested_workers in prop_oneof![Just(0usize), Just(1), 2usize..=4],
        action in terminal_actions(),
    ) {
        run_generated_lifecycle_case(requested_workers, action)?;
    }
}

fn run_generated_lifecycle_case(
    requested_workers: usize,
    action: TerminalAction,
) -> TestCaseResult {
    let runtime = Builder::new_current_thread()
        .enable_all()
        .build()
        .map_err(|error| TestCaseError::fail(error.to_string()))?;
    runtime
        .block_on(exercise_lifecycle(requested_workers, action))
        .map_err(|error| TestCaseError::fail(error.to_string()))
}

async fn exercise_lifecycle(requested_workers: usize, action: TerminalAction) -> TestResult {
    assert_normalized_worker_count(requested_workers)?;
    match action {
        TerminalAction::Graceful => graceful_shutdown(requested_workers).await,
        TerminalAction::Drop => drop_supervisor(requested_workers).await,
        TerminalAction::Abort => abort_supervisor(requested_workers).await,
    }
}

fn assert_normalized_worker_count(requested_workers: usize) -> TestResult {
    let server = WireframeServer::new(factory()).workers(requested_workers);
    let actual_workers = server.worker_count();
    let expected_workers = requested_workers.max(1);
    if actual_workers != expected_workers {
        return Err(format!(
            "worker count mismatch: requested={requested_workers}, actual={actual_workers}, \
             expected={expected_workers}"
        )
        .into());
    }
    Ok(())
}

async fn graceful_shutdown(requested_workers: usize) -> TestResult {
    let listener = unused_listener()?;
    let server = WireframeServer::new(factory())
        .workers(requested_workers)
        .bind_existing_listener(listener)?;
    let addr = server
        .local_addr()
        .ok_or_else(|| "local address missing".to_string())?;
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
        .map_err(|error| format!("ready server did not accept a connection: {error}"))?;
    drop(stream);
    shutdown_tx
        .send(())
        .map_err(|()| "server shutdown receiver was dropped")?;
    timeout(Duration::from_secs(1), handle)
        .await
        .map_err(|_| "graceful server did not complete in time")???;
    wait_for_listener_release(addr).await
}

async fn drop_supervisor(requested_workers: usize) -> TestResult {
    let listener = unused_listener()?;
    let server = WireframeServer::new(factory())
        .workers(requested_workers)
        .bind_existing_listener(listener)?;
    let addr = server
        .local_addr()
        .ok_or_else(|| "local address missing".to_string())?;
    let (ready_tx, mut ready_rx) = oneshot::channel();
    let mut supervisor = Box::pin(
        server
            .ready_signal(ready_tx)
            .run_with_shutdown(future::pending()),
    );
    #[expect(
        clippy::integer_division_remainder_used,
        reason = "tokio::select! expands to modulus internally"
    )]
    let readiness: TestResult = timeout(Duration::from_secs(1), async {
        tokio::select! {
            ready = &mut ready_rx => ready.map_err(|error| {
                format!("server dropped readiness sender: {error}").into()
            }),
            result = &mut supervisor => Err(format!(
                "server supervisor completed before readiness: {result:?}"
            ).into()),
        }
    })
    .await
    .map_err(|_| "server did not reach readiness")?;
    readiness?;
    drop(supervisor);
    wait_for_listener_release(addr).await
}

async fn abort_supervisor(requested_workers: usize) -> TestResult {
    let listener = unused_listener()?;
    let server = WireframeServer::new(factory())
        .workers(requested_workers)
        .bind_existing_listener(listener)?;
    let addr = server
        .local_addr()
        .ok_or_else(|| "local address missing".to_string())?;
    let (ready_tx, ready_rx) = oneshot::channel();
    let handle = tokio::spawn(async move {
        server
            .ready_signal(ready_tx)
            .run_with_shutdown(future::pending())
            .await
    });

    wait_for_server_readiness(ready_rx).await?;
    handle.abort();
    let join_result = timeout(Duration::from_secs(1), handle)
        .await
        .map_err(|_| "aborted server did not finish in time")?;
    let Err(error) = join_result else {
        return Err("aborted server supervisor unexpectedly completed".into());
    };
    if !error.is_cancelled() {
        return Err("server supervisor was not cancelled".into());
    }
    wait_for_listener_release(addr).await
}
