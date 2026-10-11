//! Coverage for supervisor terminal observation.

use std::{error::Error as _, future::pending, io};

#[cfg(feature = "metrics")]
use metrics::with_local_recorder;
use tokio::{runtime::Builder, sync::watch, task::JoinHandle};
use tokio_util::sync::CancellationToken;
use tracing_test::traced_test;
#[cfg(feature = "metrics")]
use wireframe_testing::ObservabilityHandle;

use super::{ServerError, ServerShutdown, observe_supervisor_termination};
#[cfg(feature = "metrics")]
use crate::metrics::SERVER_SUPERVISOR_ABNORMAL_TERMINATIONS;

/// Panics after yielding so the supervisor observer receives a join failure.
async fn panic_supervisor() -> Result<(), ServerError> {
    tokio::task::yield_now().await;
    panic!("supervisor test panic");
}

/// Runs a panicking supervisor through the terminal observation path.
async fn panicking_supervisor_outcome() -> Result<(), ServerError> {
    let supervisor = tokio::spawn(panic_supervisor());
    observe_supervisor_outcome(supervisor).await
}

/// Runs an aborted supervisor through the terminal observation path.
async fn cancelled_supervisor_outcome() -> Result<(), ServerError> {
    let supervisor = tokio::spawn(pending::<Result<(), ServerError>>());
    supervisor.abort();
    observe_supervisor_outcome(supervisor).await
}

/// Runs a supervisor returning a typed failure through terminal observation.
async fn failed_supervisor_outcome() -> Result<(), ServerError> {
    let supervisor = tokio::spawn(async {
        Err(ServerError::Accept(io::Error::other(
            "synthetic accept failure",
        )))
    });
    observe_supervisor_outcome(supervisor).await
}

/// Observes one supervisor task and awaits the public terminal result.
async fn observe_supervisor_outcome(
    supervisor: JoinHandle<Result<(), ServerError>>,
) -> Result<(), ServerError> {
    let (terminal_tx, terminal_rx) = watch::channel(None);
    observe_supervisor_termination(supervisor, terminal_tx).await;

    ServerShutdown::new(CancellationToken::new(), terminal_rx)
        .drained()
        .await
}

/// Builds the current-thread runtime used by synchronous observability tests.
fn observation_runtime() -> io::Result<tokio::runtime::Runtime> {
    Builder::new_current_thread().enable_all().build()
}

/// Checks the typed abnormal outcome and returns it for source-chain checks.
fn require_abnormal_termination(
    result: Result<(), ServerError>,
    expected_message: &str,
) -> io::Result<ServerError> {
    match result {
        Err(error)
            if matches!(
                &error,
                ServerError::AbnormalTermination { message, .. }
                    if message.contains(expected_message)
            ) =>
        {
            Ok(error)
        }
        Err(error) => Err(io::Error::other(format!(
            "unexpected supervisor error: {error}"
        ))),
        Ok(()) => Err(io::Error::other(
            "failed supervisor unexpectedly completed cleanly",
        )),
    }
}

#[traced_test]
#[test]
fn panicking_supervisor_reports_terminal_error_and_observability() {
    #[cfg(feature = "metrics")]
    let mut observability = ObservabilityHandle::new();
    let runtime = observation_runtime().expect("build current-thread runtime");
    #[cfg(feature = "metrics")]
    let result = with_local_recorder(observability.recorder(), || {
        runtime.block_on(panicking_supervisor_outcome())
    });
    #[cfg(not(feature = "metrics"))]
    let result = runtime.block_on(panicking_supervisor_outcome());

    let _error = require_abnormal_termination(result, "supervisor test panic")
        .expect("expected abnormal termination");
    #[cfg(feature = "metrics")]
    {
        observability.snapshot();
        observability
            .assert_counter(SERVER_SUPERVISOR_ABNORMAL_TERMINATIONS, [], 1)
            .expect("abnormal termination metric missing");
    }
    logs_assert(|lines: &[&str]| {
        lines
            .iter()
            .any(|line| {
                line.contains("server_supervisor_abnormal_termination")
                    && line.contains("supervisor test panic")
            })
            .then_some(())
            .ok_or_else(|| "abnormal termination trace event missing".to_owned())
    });
}

#[traced_test]
#[test]
fn cancelled_supervisor_reports_error_without_abnormal_termination_metric() {
    #[cfg(feature = "metrics")]
    let mut observability = ObservabilityHandle::new();
    let runtime = observation_runtime().expect("build current-thread runtime");
    #[cfg(feature = "metrics")]
    let result = with_local_recorder(observability.recorder(), || {
        runtime.block_on(cancelled_supervisor_outcome())
    });
    #[cfg(not(feature = "metrics"))]
    let result = runtime.block_on(cancelled_supervisor_outcome());

    let _error =
        require_abnormal_termination(result, "cancelled").expect("expected abnormal termination");
    #[cfg(feature = "metrics")]
    {
        observability.snapshot();
        observability
            .assert_counter(SERVER_SUPERVISOR_ABNORMAL_TERMINATIONS, [], 0)
            .expect("cancelled supervisor incremented the abnormal termination metric");
    }
    logs_assert(|lines: &[&str]| {
        let has_cancellation_event = lines
            .iter()
            .any(|line| line.contains("server_supervisor_cancelled"));
        let has_abnormal_termination_event = lines
            .iter()
            .any(|line| line.contains("server_supervisor_abnormal_termination"));

        (has_cancellation_event && !has_abnormal_termination_event)
            .then_some(())
            .ok_or_else(|| "supervisor cancellation event classification was incorrect".to_owned())
    });
}

#[test]
fn returned_supervisor_error_is_preserved_as_the_abnormal_error_source() {
    let error = require_abnormal_termination(
        observation_runtime()
            .expect("build current-thread runtime")
            .block_on(failed_supervisor_outcome()),
        "accept error: synthetic accept failure",
    )
    .expect("expected abnormal termination");
    let source = error
        .source()
        .expect("returned error source must be retained");
    assert_eq!(source.to_string(), "accept error: synthetic accept failure");
    assert_eq!(
        source
            .source()
            .expect("underlying I/O error must remain in the source chain")
            .to_string(),
        "synthetic accept failure"
    );
}
