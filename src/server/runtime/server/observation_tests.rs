//! Coverage for supervisor terminal observation.

use std::future::pending;

use metrics::with_local_recorder;
use tokio::{runtime::Builder, sync::watch};
use tokio_util::sync::CancellationToken;
use tracing_test::traced_test;
use wireframe_testing::ObservabilityHandle;

use super::{ServerError, ServerShutdown, observe_supervisor_termination};
use crate::metrics::SERVER_SUPERVISOR_ABNORMAL_TERMINATIONS;

async fn panic_supervisor() -> Result<(), ServerError> {
    tokio::task::yield_now().await;
    panic!("supervisor test panic");
}

#[traced_test]
#[test]
fn panicking_supervisor_reports_terminal_error_and_observability() {
    let mut observability = ObservabilityHandle::new();
    let runtime = Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("build current-thread runtime");
    let result = with_local_recorder(observability.recorder(), || {
        runtime.block_on(async {
            let (terminal_tx, terminal_rx) = watch::channel(None);
            let supervisor = tokio::spawn(panic_supervisor());
            observe_supervisor_termination(supervisor, terminal_tx).await;

            ServerShutdown::new(CancellationToken::new(), terminal_rx)
                .drained()
                .await
        })
    });

    assert!(matches!(
        result,
        Err(ServerError::AbnormalTermination { ref message })
            if message.contains("supervisor test panic")
    ));
    observability.snapshot();
    observability
        .assert_counter(SERVER_SUPERVISOR_ABNORMAL_TERMINATIONS, [], 1)
        .expect("abnormal termination metric missing");
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
    let mut observability = ObservabilityHandle::new();
    let runtime = Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("build current-thread runtime");
    let result = with_local_recorder(observability.recorder(), || {
        runtime.block_on(async {
            let (terminal_tx, terminal_rx) = watch::channel(None);
            let supervisor = tokio::spawn(pending::<Result<(), ServerError>>());
            supervisor.abort();
            observe_supervisor_termination(supervisor, terminal_tx).await;

            ServerShutdown::new(CancellationToken::new(), terminal_rx)
                .drained()
                .await
        })
    });

    assert!(matches!(
        result,
        Err(ServerError::AbnormalTermination { ref message })
            if message.contains("cancelled")
    ));
    observability.snapshot();
    observability
        .assert_counter(SERVER_SUPERVISOR_ABNORMAL_TERMINATIONS, [], 0)
        .expect("cancelled supervisor incremented the abnormal termination metric");
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
