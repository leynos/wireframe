//! Startup cancellation coverage for the server runtime.

#[cfg(feature = "metrics")]
use std::io;

#[cfg(feature = "metrics")]
use metrics_util::debugging::{DebugValue, DebuggingRecorder};
use tokio::{
    sync::oneshot,
    time::{Duration, timeout},
};

use super::{WireframeServer, test_support::preparation_factory};
use crate::{app::WireframeApp, server::test_util::free_listener};
#[cfg(feature = "metrics")]
use crate::{
    metrics::{SERVER_STARTUP_DURATION, SERVER_STARTUP_FAILURES},
    server::ServerError,
};

/// Shutdown cancels a blocked preparation without publishing readiness.
#[tokio::test]
async fn shutdown_interrupts_blocked_preparation_before_readiness()
-> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let (entered, _barrier, factory) = preparation_factory();
    let server = WireframeServer::new(factory).bind_existing_listener(free_listener()?)?;
    let (ready_tx, ready_rx) = oneshot::channel();
    let (shutdown_tx, shutdown_rx) = oneshot::channel();
    let server_task = tokio::spawn(async move {
        server
            .ready_signal(ready_tx)
            .run_with_shutdown(async {
                let _ = shutdown_rx.await;
            })
            .await
    });

    timeout(Duration::from_secs(1), entered.notified())
        .await
        .map_err(|_| "application preparation did not start")?;
    shutdown_tx
        .send(())
        .map_err(|()| "server shutdown receiver was dropped")?;
    timeout(Duration::from_secs(1), server_task)
        .await
        .map_err(|_| "blocked preparation ignored shutdown")???;
    if ready_rx.await.is_ok() {
        return Err("server signalled readiness after interrupted preparation".into());
    }
    Ok(())
}

/// Factory startup failures emit one bounded observability counter.
#[cfg(feature = "metrics")]
#[test]
fn factory_failure_records_a_bounded_startup_metric() {
    let recorder = DebuggingRecorder::new();
    let snapshotter = recorder.snapshotter();
    let result = metrics::with_local_recorder(&recorder, || {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("test runtime should build");
        runtime.block_on(async {
            let factory = || -> Result<WireframeApp, io::Error> {
                Err(io::Error::other("application construction failed"))
            };
            let server = WireframeServer::new(factory)
                .bind_existing_listener(free_listener().expect("test listener should bind"))
                .expect("test server should bind");
            server.run_with_shutdown(std::future::pending()).await
        })
    });

    assert!(matches!(result, Err(ServerError::FactoryBuild(_))));
    let metrics = snapshotter.snapshot().into_vec();
    assert!(metrics.iter().any(|(key, _, _, value)| {
        key.key().name() == SERVER_STARTUP_FAILURES
            && key
                .key()
                .labels()
                .any(|label| label.key() == "stage" && label.value() == "factory_build")
            && matches!(value, DebugValue::Counter(count) if *count == 1)
    }));
    assert!(metrics.iter().any(|(key, _, _, value)| {
        key.key().name() == SERVER_STARTUP_DURATION
            && key
                .key()
                .labels()
                .any(|label| label.key() == "outcome" && label.value() == "factory_build")
            && matches!(value, DebugValue::Histogram(_))
    }));
}
