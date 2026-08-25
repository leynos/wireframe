//! Lifecycle-observability tests for server-supervisor cancellation.
//!
//! Metrics use a current-thread runtime because the local recorder must remain
//! installed while accept loops observe cancellation and emit their exits.
#![cfg(not(loom))]

use std::future;

use metrics::with_local_recorder;
use tokio::{
    runtime::Builder,
    sync::oneshot,
    time::{Duration, timeout},
};
use tracing::Instrument;
use tracing_test::traced_test;
use wireframe::{
    metrics::{SERVER_ACCEPT_LOOPS_EXITED, SERVER_SUPERVISOR_CANCELLATIONS},
    server::WireframeServer,
};
use wireframe_testing::{
    ObservabilityHandle,
    TestResult,
    factory,
    unused_listener,
    wait_for_listener_release,
    wait_for_server_readiness,
};

const WORKERS: usize = 2;

macro_rules! assert_trace_event {
    ($event:expr, $reason:expr) => {
        logs_assert(|lines: &[&str]| {
            let captured = lines.join("\\n");
            if !lines
                .iter()
                .any(|line| line.contains($event) && line.contains($reason))
            {
                return Err(format!(
                    "trace event {:?} with reason {:?} was not captured; captured logs: {captured}",
                    $event, $reason
                ));
            }
            Ok(())
        });
    };
}

#[traced_test]
#[test]
fn graceful_shutdown_emits_lifecycle_metrics_and_trace() -> TestResult {
    let mut observability = ObservabilityHandle::new();
    let span = tracing::Span::current();
    run_with_local_metrics(&observability, graceful_shutdown().instrument(span))?;
    observability.snapshot();

    observability
        .assert_counter(SERVER_SUPERVISOR_CANCELLATIONS, [("reason", "graceful")], 1)
        .map_err(|error| format!("graceful supervisor cancellation metric missing: {error}"))?;
    observability
        .assert_counter(
            SERVER_ACCEPT_LOOPS_EXITED,
            [("reason", "graceful")],
            WORKERS as u64,
        )
        .map_err(|error| format!("graceful accept-loop exit metrics missing: {error}"))?;
    assert_trace_event!("server_supervisor_cancellation", "graceful");
    assert_trace_event!("server_accept_loop_exited", "graceful");
    Ok(())
}

#[traced_test]
#[test]
fn dropped_supervisor_emits_lifecycle_metrics_and_trace() -> TestResult {
    let mut observability = ObservabilityHandle::new();
    let span = tracing::Span::current();
    run_with_local_metrics(&observability, drop_supervisor().instrument(span))?;
    observability.snapshot();

    assert_dropped_metrics(&observability)?;
    assert_trace_event!("server_supervisor_cancellation", "dropped");
    assert_trace_event!("server_accept_loop_exited", "dropped");
    Ok(())
}

#[test]
fn aborted_supervisor_emits_dropped_lifecycle_metrics() -> TestResult {
    let mut observability = ObservabilityHandle::new();
    run_with_local_metrics(&observability, abort_supervisor())?;
    observability.snapshot();

    assert_dropped_metrics(&observability)
}

fn run_with_local_metrics<F>(observability: &ObservabilityHandle, future: F) -> TestResult
where
    F: Future<Output = TestResult>,
{
    let runtime = Builder::new_current_thread().enable_all().build()?;
    with_local_recorder(observability.recorder(), || runtime.block_on(future))
}

fn assert_dropped_metrics(observability: &ObservabilityHandle) -> TestResult {
    observability
        .assert_counter(SERVER_SUPERVISOR_CANCELLATIONS, [("reason", "dropped")], 1)
        .map_err(|error| format!("dropped supervisor cancellation metric missing: {error}"))?;
    observability
        .assert_counter(
            SERVER_ACCEPT_LOOPS_EXITED,
            [("reason", "dropped")],
            WORKERS as u64,
        )
        .map_err(|error| format!("dropped accept-loop exit metrics missing: {error}"))?;
    Ok(())
}

async fn graceful_shutdown() -> TestResult {
    let listener = unused_listener()?;
    let server = WireframeServer::new(factory())
        .workers(WORKERS)
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
    shutdown_tx
        .send(())
        .map_err(|()| "server shutdown receiver was dropped")?;
    let server_result = timeout(Duration::from_secs(1), handle)
        .await
        .map_err(|_| "graceful server did not complete in time")??;
    server_result?;
    wait_for_listener_release(addr).await
}

async fn drop_supervisor() -> TestResult {
    let listener = unused_listener()?;
    let server = WireframeServer::new(factory())
        .workers(WORKERS)
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

async fn abort_supervisor() -> TestResult {
    let listener = unused_listener()?;
    let server = WireframeServer::new(factory())
        .workers(WORKERS)
        .bind_existing_listener(listener)?;
    let addr = server
        .local_addr()
        .ok_or_else(|| "local address missing".to_string())?;
    let (ready_tx, ready_rx) = oneshot::channel();
    let span = tracing::Span::current();
    let handle = tokio::spawn(
        async move {
            server
                .ready_signal(ready_tx)
                .run_with_shutdown(future::pending())
                .await
        }
        .instrument(span),
    );

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
