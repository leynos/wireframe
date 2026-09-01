//! Observability coverage for prepared application transitions.
#![cfg(feature = "metrics")]

use metrics::with_local_recorder;
use tokio::runtime::Builder;
use wireframe::{
    app::{Envelope, WireframeApp},
    metrics::{
        APPLICATION_PREPARATION_DURATION,
        APPLICATION_PREPARATIONS,
        PREPARED_CONNECTION_USES,
    },
    serializer::BincodeSerializer,
};
use wireframe_testing::{ObservabilityHandle, TestResult, drive_prepared_with_frames};

/// Verify preparation and prepared-connection metrics use bounded series.
#[test]
fn prepared_application_metrics_record_outcome_duration_and_use() -> TestResult<()> {
    let mut observability = ObservabilityHandle::new();
    let runtime = Builder::new_current_thread().enable_all().build()?;
    with_local_recorder(observability.recorder(), || {
        runtime.block_on(async {
            let app: WireframeApp<BincodeSerializer, (), Envelope> = WireframeApp::new()?;
            let prepared = app
                .prepare()
                .await
                .map_err(|error| Box::new(error) as Box<dyn std::error::Error + Send + Sync>)?;
            drive_prepared_with_frames(&prepared, Vec::new()).await?;
            drive_prepared_with_frames(&prepared, Vec::new()).await?;
            Ok::<(), wireframe_testing::TestError>(())
        })
    })?;

    observability.snapshot();
    observability
        .assert_counter(APPLICATION_PREPARATIONS, [("outcome", "success")], 1)
        .map_err(|error| format!("preparation success metric missing: {error}"))?;
    observability
        .assert_counter(APPLICATION_PREPARATIONS, [("outcome", "failure")], 0)
        .map_err(|error| format!("preparation failure series should remain absent: {error}"))?;
    observability
        .assert_counter(PREPARED_CONNECTION_USES, [], 2)
        .map_err(|error| format!("prepared-connection use metric missing: {error}"))?;
    observability
        .assert_histogram_recorded(APPLICATION_PREPARATION_DURATION, [("outcome", "success")])
        .map_err(|error| format!("preparation duration metric missing: {error}"))?;
    Ok(())
}
