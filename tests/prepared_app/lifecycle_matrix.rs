//! Lifecycle matrix for connection-local finalization.
//!
//! Each case drives a prepared application far enough for setup to produce a
//! non-`Clone` identity token, terminates the connection through one of the
//! documented outcomes, and checks that teardown received *that* token exactly
//! once. Setup failures and a setup that never completes must invoke teardown
//! zero times.

use std::{
    panic::AssertUnwindSafe,
    sync::{
        Arc,
        atomic::{AtomicU64, AtomicUsize, Ordering},
    },
    time::Duration,
};

use futures::FutureExt;
use rstest::rstest;
use tokio::{sync::Notify, time::timeout};
use wireframe::{
    app::{Envelope, PreparedApp, WireframeApp},
    serializer::BincodeSerializer,
};
use wireframe_testing::TestResult;

use super::{
    fallible_check::check,
    fallible_check_equal::check_equal,
    handler,
    lifecycle_support::{
        OwnedToken,
        Recorder,
        ScriptedStream,
        TerminalOutcome,
        assert_single_lifecycle,
        counting_teardown,
        gated_setup,
        lifecycle_app,
        panicking_handler,
        panicking_setup,
        panicking_teardown,
        stream_for,
    },
};

/// Prepared form of the lifecycle application.
type LifecyclePrepared = PreparedApp<BincodeSerializer, OwnedToken, Envelope>;

/// Every terminal outcome after successful setup finalizes exactly once.
#[rstest]
#[case::clean_eof(TerminalOutcome::CleanEof)]
#[case::decode_error(TerminalOutcome::DecodeError)]
#[case::protocol_error(TerminalOutcome::ProtocolError)]
#[case::read_error(TerminalOutcome::ReadError)]
#[case::write_error(TerminalOutcome::WriteError)]
#[case::handler_panic(TerminalOutcome::HandlerPanic)]
#[tokio::test]
async fn teardown_runs_once_per_terminal_outcome(
    #[case] outcome: TerminalOutcome,
) -> TestResult<()> {
    let recorder = Recorder::default();
    let route_handler = match outcome {
        TerminalOutcome::HandlerPanic => panicking_handler(),
        _ => handler(),
    };
    let prepared: LifecyclePrepared =
        lifecycle_app(recorder.clone(), Arc::new(AtomicU64::new(0)), route_handler)?
            .prepare()
            .await
            .map_err(|error| -> Box<dyn std::error::Error + Send + Sync> { Box::new(error) })?;
    let stream = stream_for(outcome)?;

    let result = AssertUnwindSafe(prepared.handle_connection_result(stream))
        .catch_unwind()
        .await;

    match outcome {
        TerminalOutcome::HandlerPanic => {
            check(result.is_err(), "the handler panic should propagate")?;
        }
        TerminalOutcome::CleanEof => {
            result
                .map_err(|_| "clean end of stream should not panic")?
                .map_err(|error| format!("clean end of stream failed: {error}"))?;
        }
        _ => {
            let processing = result.map_err(|_| "error outcome should not panic")?;
            check(
                processing.is_err(),
                "the connection should end with a processing error",
            )?;
        }
    }
    assert_single_lifecycle(&recorder)
}

/// A setup panic must not invoke teardown.
#[tokio::test]
async fn setup_panic_runs_teardown_zero_times() -> TestResult<()> {
    let teardowns = Arc::new(AtomicUsize::new(0));
    let prepared = WireframeApp::<BincodeSerializer, (), Envelope>::new()?
        .on_connection_setup(panicking_setup())?
        .on_connection_teardown(counting_teardown(Arc::clone(&teardowns)))?
        .prepare()
        .await
        .map_err(|error| -> Box<dyn std::error::Error + Send + Sync> { Box::new(error) })?;

    let result =
        AssertUnwindSafe(prepared.handle_connection_result(ScriptedStream::new(Vec::new())))
            .catch_unwind()
            .await;

    check(result.is_err(), "the setup panic should propagate")?;
    Ok(check_equal(
        &teardowns.load(Ordering::SeqCst),
        &0usize,
        "teardown must not run when setup panicked",
    )?)
}

/// A setup that never completes and is dropped must not invoke teardown.
#[tokio::test]
async fn dropped_setup_runs_teardown_zero_times() -> TestResult<()> {
    let teardowns = Arc::new(AtomicUsize::new(0));
    let gate = Arc::new(Notify::new());
    let prepared = WireframeApp::<BincodeSerializer, (), Envelope>::new()?
        .on_connection_setup(gated_setup(gate))?
        .on_connection_teardown(counting_teardown(Arc::clone(&teardowns)))?
        .prepare()
        .await
        .map_err(|error| -> Box<dyn std::error::Error + Send + Sync> { Box::new(error) })?;

    let result = timeout(
        Duration::from_millis(50),
        prepared.handle_connection_result(ScriptedStream::new(Vec::new())),
    )
    .await;

    check(result.is_err(), "gated setup should time out")?;
    Ok(check_equal(
        &teardowns.load(Ordering::SeqCst),
        &0usize,
        "teardown must not run when setup never completed",
    )?)
}

/// A teardown panic propagates once and does not poison later connections.
#[tokio::test]
async fn teardown_panic_is_isolated_to_one_connection() -> TestResult<()> {
    let invocations = Arc::new(AtomicUsize::new(0));
    let prepared = WireframeApp::<BincodeSerializer, (), Envelope>::new()?
        .on_connection_setup(|| async {})?
        .on_connection_teardown(panicking_teardown(Arc::clone(&invocations)))?
        .prepare()
        .await
        .map_err(|error| -> Box<dyn std::error::Error + Send + Sync> { Box::new(error) })?;

    let first =
        AssertUnwindSafe(prepared.handle_connection_result(ScriptedStream::new(Vec::new())))
            .catch_unwind()
            .await;
    check(first.is_err(), "the teardown panic should propagate")?;
    check_equal(
        &invocations.load(Ordering::SeqCst),
        &1usize,
        "the panicking teardown should run once",
    )?;

    prepared
        .handle_connection_result(ScriptedStream::new(Vec::new()))
        .await?;
    Ok(check_equal(
        &invocations.load(Ordering::SeqCst),
        &2usize,
        "a healthy connection should still finalize afterwards",
    )?)
}
