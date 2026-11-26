//! Property tests for the server lifecycle harness invariants.
//!
//! These cover RFC 0001 §7.5: repeated and cancelled shutdown calls converge,
//! a schedule of connections leaves nothing behind, and parallel handles are
//! independent. Each case runs on a fresh runtime inside a ten-second bound,
//! so a hang becomes a printable, shrinkable failure instead of a stuck suite.
#![cfg(not(loom))]
#![forbid(unsafe_code)]

use std::{
    future::Future,
    sync::{Arc, atomic::AtomicUsize},
    time::Duration,
};

use futures::poll;
use proptest::{
    prelude::*,
    test_runner::{TestCaseError, TestCaseResult},
};
use rstest::rstest;
use tokio::{
    net::TcpStream,
    runtime::Builder,
    time::{Instant, timeout},
};
use wireframe::server::WireframeServer;
use wireframe_testing::{
    RunningWireframeServer,
    ShutdownOutcome,
    TestError,
    TestResult,
    echo_app_factory,
    spawn_wireframe_server,
    wait_for_listener_release,
};

/// Internal bound for one generated case.
const CASE_TIMEOUT: Duration = Duration::from_secs(10);
/// Bound for a final drain to complete once its causes are gone.
const FINAL_DRAIN: Duration = Duration::from_secs(5);

/// One operation in a generated shutdown trace.
#[derive(Clone, Copy, Debug)]
enum TraceOperation {
    /// Call `shutdown()` to completion.
    Shutdown,
    /// Poll `shutdown()` once, then drop the future.
    CancelledShutdown,
}

/// How a generated trace ends.
#[derive(Clone, Copy, Debug)]
enum TerminalAction {
    /// Call `shutdown()` until the drain completes.
    Shutdown,
    /// Drop the handle.
    Drop,
}

/// Strategy for one trace operation.
fn trace_operation_strategy() -> impl Strategy<Value = TraceOperation> {
    prop_oneof![
        Just(TraceOperation::Shutdown),
        Just(TraceOperation::CancelledShutdown),
    ]
}

/// Strategy for the terminal action.
fn terminal_action_strategy() -> impl Strategy<Value = TerminalAction> {
    prop_oneof![Just(TerminalAction::Shutdown), Just(TerminalAction::Drop)]
}

/// Run one property case on a fresh runtime, failing instead of hanging.
fn run_bounded_case(case: impl Future<Output = TestCaseResult>) -> TestCaseResult {
    let runtime = Builder::new_current_thread()
        .enable_all()
        .build()
        .map_err(|error| TestCaseError::fail(error.to_string()))?;
    runtime.block_on(async {
        timeout(CASE_TIMEOUT, case)
            .await
            .map_err(|_| TestCaseError::fail("case exceeded 10 s"))?
    })
}

/// Spawn one harness server for a generated case.
async fn spawn_case_server() -> Result<(RunningWireframeServer, std::net::SocketAddr), TestCaseError>
{
    let counter = Arc::new(AtomicUsize::new(0));
    let server = WireframeServer::new(echo_app_factory(&counter)).workers(1);
    let handle = spawn_wireframe_server(server)
        .await
        .map_err(|error| TestCaseError::fail(format!("server spawn failed: {error}")))?;
    let addr = handle.local_addr();
    Ok((handle, addr))
}

/// Poll one shutdown future once, reporting a result that was already ready.
///
/// After the drain has concluded, `shutdown()` resolves on the first poll, so
/// cancelling a future only makes sense while it is still pending. Both
/// outcomes are legitimate; the returned result lets the trace keep its
/// "later calls never regress" invariant.
async fn cancel_one_shutdown(handle: &mut RunningWireframeServer) -> Option<TestResult<()>> {
    let pending = handle.shutdown();
    let mut pinned = std::pin::pin!(pending);
    let first_poll = poll!(pinned.as_mut());
    match first_poll {
        std::task::Poll::Ready(result) => Some(result),
        std::task::Poll::Pending => None,
    }
}

/// Drive a generated trace and its terminal action, asserting INV-5.
async fn exercise_shutdown_trace(
    trace: Vec<TraceOperation>,
    terminal: TerminalAction,
) -> TestCaseResult {
    let (mut handle, addr) = spawn_case_server().await?;
    let mut saw_success = false;

    for (index, operation) in trace.iter().enumerate() {
        let outcome_before = handle.shutdown_outcome();
        if let Some(result) = apply_operation(&mut handle, *operation).await {
            check_no_regression(index, saw_success, &result)?;
            saw_success |= result.is_ok();
        }
        if outcome_before == Some(ShutdownOutcome::Completed) {
            prop_assert_eq!(
                handle.shutdown_outcome(),
                Some(ShutdownOutcome::Completed),
                "call {} regressed the completed outcome",
                index
            );
        }
    }

    match terminal {
        TerminalAction::Shutdown => finish_with_shutdown(&mut handle, addr).await,
        TerminalAction::Drop => finish_with_drop(handle, addr).await,
    }
}

/// Apply one trace operation, returning a result that was already ready.
async fn apply_operation(
    handle: &mut RunningWireframeServer,
    operation: TraceOperation,
) -> Option<TestResult<()>> {
    match operation {
        TraceOperation::Shutdown => Some(handle.shutdown().await),
        TraceOperation::CancelledShutdown => cancel_one_shutdown(handle).await,
    }
}

/// Require that a later call never regresses a completed outcome.
fn check_no_regression(
    index: usize,
    saw_success: bool,
    result: &TestResult<()>,
) -> Result<(), TestCaseError> {
    if saw_success && result.is_err() {
        return Err(TestCaseError::fail(format!(
            "call {index} regressed after a completed shutdown: {result:?}"
        )));
    }
    Ok(())
}

/// Drain the handle to completion and require a released listener.
///
/// Every attempt is checked against the preceding state, so a transient
/// regression of a completed outcome fails even when a later retry succeeds.
async fn finish_with_shutdown(
    handle: &mut RunningWireframeServer,
    addr: std::net::SocketAddr,
) -> TestCaseResult {
    let deadline = Instant::now() + FINAL_DRAIN;
    loop {
        let was_completed = handle.shutdown_outcome() == Some(ShutdownOutcome::Completed);
        let result = handle.shutdown().await;
        if was_completed {
            prop_assert!(
                result.is_ok(),
                "the final drain regressed a completed outcome: {result:?}"
            );
        }
        let outcome = handle.shutdown_outcome();
        if was_completed {
            prop_assert_eq!(outcome, Some(ShutdownOutcome::Completed));
        }
        if result.is_ok() {
            prop_assert_eq!(outcome, Some(ShutdownOutcome::Completed));
            break;
        }
        prop_assert!(
            Instant::now() < deadline,
            "the final drain never completed at {addr}"
        );
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    assert_refused(addr).await
}

/// Drop the handle and require a released listener.
async fn finish_with_drop(
    handle: RunningWireframeServer,
    addr: std::net::SocketAddr,
) -> TestCaseResult {
    drop(handle);
    wait_for_listener_release(addr)
        .await
        .map_err(|error| TestCaseError::fail(format!("listener not released: {error}")))
}

/// Require the address to refuse a connection.
async fn assert_refused(addr: std::net::SocketAddr) -> Result<(), TestCaseError> {
    match TcpStream::connect(addr).await {
        Err(error) if error.kind() == std::io::ErrorKind::ConnectionRefused => Ok(()),
        Ok(stream) => {
            drop(stream);
            Err(TestCaseError::fail(format!(
                "listener at {addr} accepted a connection after the drain"
            )))
        }
        Err(error) => Err(TestCaseError::fail(error.to_string())),
    }
}

proptest! {
    #![proptest_config(ProptestConfig {
        cases: 16,
        max_shrink_time: 30_000,
        ..ProptestConfig::default()
    })]

    #[test]
    fn shutdown_traces_converge(
        trace in prop::collection::vec(trace_operation_strategy(), 0..=5),
        terminal in terminal_action_strategy(),
    ) {
        run_bounded_case(exercise_shutdown_trace(trace, terminal))?;
    }

    #[test]
    fn connect_schedules_leave_nothing_behind(
        schedule in prop::collection::vec(any::<bool>(), 0..=3),
    ) {
        run_bounded_case(exercise_connect_schedule(schedule))?;
    }

    #[test]
    fn parallel_handles_are_independent(
        pool_size in 2usize..=4,
        shutdown_index in any::<usize>(),
    ) {
        run_bounded_case(exercise_parallel_pool(pool_size, shutdown_index))?;
    }
}

/// Connect zero or more peers, then drain, asserting INV-6.
async fn exercise_connect_schedule(schedule: Vec<bool>) -> TestCaseResult {
    let (mut handle, addr) = spawn_case_server().await?;
    let mut open_peers = Vec::new();

    for keep_open in &schedule {
        let stream = TcpStream::connect(addr)
            .await
            .map_err(|error| TestCaseError::fail(format!("connect failed: {error}")))?;
        if *keep_open {
            open_peers.push(stream);
        } else {
            drop(stream);
        }
    }

    timeout(FINAL_DRAIN, handle.shutdown())
        .await
        .map_err(|_| TestCaseError::fail("the drain did not finish within its bound"))?
        .map_err(|error| TestCaseError::fail(format!("shutdown failed: {error}")))?;
    prop_assert_eq!(handle.shutdown_outcome(), Some(ShutdownOutcome::Completed));
    assert_refused(addr).await?;
    drop(open_peers);
    Ok(())
}

/// Spawn a pool of handles, stop one, and require the rest to accept INV-7.
async fn exercise_parallel_pool(pool_size: usize, shutdown_index: usize) -> TestCaseResult {
    let mut handles = spawn_pool(pool_size).await?;
    let addresses = distinct_addresses(&handles)?;
    let selector = shutdown_index
        .checked_rem(pool_size)
        .ok_or_else(|| TestCaseError::fail("the pool size must be non-zero"))?;

    stop_selected(&mut handles, &addresses, selector).await?;
    check_survivors(&handles, selector).await?;
    stop_all(&mut handles).await
}

/// Spawn `pool_size` independent handles.
async fn spawn_pool(pool_size: usize) -> Result<Vec<RunningWireframeServer>, TestCaseError> {
    let mut handles = Vec::with_capacity(pool_size);
    for _ in 0..pool_size {
        let (handle, _addr) = spawn_case_server().await?;
        handles.push(handle);
    }
    Ok(handles)
}

/// Collect the pool's addresses and require them to be distinct.
fn distinct_addresses(
    handles: &[RunningWireframeServer],
) -> Result<Vec<std::net::SocketAddr>, TestCaseError> {
    let addresses: Vec<std::net::SocketAddr> = handles
        .iter()
        .map(RunningWireframeServer::local_addr)
        .collect();
    let mut unique = addresses.clone();
    unique.sort();
    unique.dedup();
    if unique.len() != addresses.len() {
        return Err(TestCaseError::fail(format!(
            "handles must bind distinct listeners: {addresses:?}"
        )));
    }
    Ok(addresses)
}

/// Stop the selected handle and require its address to be refused.
async fn stop_selected(
    handles: &mut [RunningWireframeServer],
    addresses: &[std::net::SocketAddr],
    selector: usize,
) -> TestCaseResult {
    let stopped_addr = addresses
        .get(selector)
        .copied()
        .ok_or_else(|| TestCaseError::fail("the selector must address the pool"))?;
    let stopped = handles
        .get_mut(selector)
        .ok_or_else(|| TestCaseError::fail("the selector must address the pool"))?;
    timeout(FINAL_DRAIN, stopped.shutdown())
        .await
        .map_err(|_| TestCaseError::fail("the selected handle did not stop"))?
        .map_err(|error| TestCaseError::fail(format!("shutdown failed: {error}")))?;
    assert_refused(stopped_addr).await
}

/// Require every handle except the stopped one to accept a connection.
async fn check_survivors(handles: &[RunningWireframeServer], selector: usize) -> TestCaseResult {
    let mut survivors = 0_usize;
    for (index, handle) in handles.iter().enumerate() {
        if index == selector {
            continue;
        }
        survivors += 1;
        TcpStream::connect(handle.local_addr())
            .await
            .map_err(|error| TestCaseError::fail(format!("survivor refused a peer: {error}")))?;
    }
    prop_assert!(survivors >= 1, "at least one handle must survive the stop");
    Ok(())
}

/// Drain every remaining handle.
async fn stop_all(handles: &mut [RunningWireframeServer]) -> TestCaseResult {
    for handle in handles.iter_mut() {
        timeout(FINAL_DRAIN, handle.shutdown())
            .await
            .map_err(|_| TestCaseError::fail("a survivor did not stop"))?
            .map_err(|error| TestCaseError::fail(format!("shutdown failed: {error}")))?;
    }
    Ok(())
}

/// Named witness traces, so generation alone cannot leave a class untested.
#[rstest]
#[case(Vec::new(), TerminalAction::Drop)]
#[case(vec![TraceOperation::Shutdown], TerminalAction::Drop)]
#[case(vec![TraceOperation::CancelledShutdown], TerminalAction::Shutdown)]
#[test]
fn named_witness_traces_converge(
    #[case] trace: Vec<TraceOperation>,
    #[case] terminal: TerminalAction,
) -> TestResult {
    run_bounded_case(exercise_shutdown_trace(trace, terminal))
        .map_err(|error| TestError::Msg(format!("{error}")))
}

/// A peer held open across the drain is the INV-6 class generation can miss.
#[test]
fn named_idle_peer_witness_leaves_nothing_behind() -> TestResult {
    run_bounded_case(exercise_connect_schedule(vec![true]))
        .map_err(|error| TestError::Msg(format!("{error}")))
}
