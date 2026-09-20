//! Startup cancellation coverage for the server runtime.

#[cfg(feature = "metrics")]
use std::io;
use std::sync::Arc;
#[cfg(feature = "metrics")]
use std::time::Instant;

#[cfg(feature = "metrics")]
use metrics_util::debugging::{DebugValue, DebuggingRecorder, Snapshotter};
use tokio::{
    sync::{Barrier, Notify, oneshot},
    time::{Duration, timeout},
};

#[cfg(feature = "metrics")]
use super::startup::{prepare_application_or_shutdown, prepare_or_shutdown};
use super::{WireframeServer, test_support::PreparationBarrier};
use crate::{
    app::{Envelope, Handler, PrepareError, WireframeApp},
    server::test_util::free_listener,
};
#[cfg(feature = "metrics")]
use crate::{
    metrics::{SERVER_STARTUP_DURATION, SERVER_STARTUP_FAILURES},
    server::ServerError,
};

/// Shutdown cancels a blocked preparation without publishing readiness.
#[tokio::test]
async fn shutdown_interrupts_blocked_preparation_before_readiness()
-> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let entered = Arc::new(Notify::new());
    let barrier = Arc::new(Barrier::new(2));
    let handler: Handler<Envelope> = Arc::new(|_: &Envelope| Box::pin(async {}));
    let factory = {
        let entered = Arc::clone(&entered);
        let barrier = Arc::clone(&barrier);
        move || -> Result<WireframeApp, crate::WireframeError> {
            WireframeApp::new()?
                .route(1, Arc::clone(&handler))?
                .wrap(PreparationBarrier {
                    entered: Arc::clone(&entered),
                    barrier: Arc::clone(&barrier),
                })
        }
    };
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
    assert_factory_failure_startup_metrics(&snapshotter);
}

/// Assert the bounded startup metrics emitted for a factory construction failure.
#[cfg(feature = "metrics")]
fn assert_factory_failure_startup_metrics(snapshotter: &Snapshotter) {
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

#[cfg(feature = "metrics")]
fn assert_startup_duration_outcome(snapshotter: &Snapshotter, expected_outcome: &str) {
    let metrics = snapshotter.snapshot().into_vec();
    assert!(metrics.iter().any(|(key, _, _, value)| {
        key.key().name() == SERVER_STARTUP_DURATION
            && key.key().labels().count() == 1
            && key
                .key()
                .labels()
                .any(|label| label.key() == "outcome" && label.value() == expected_outcome)
            && matches!(value, DebugValue::Histogram(_))
    }));
}

#[cfg(feature = "metrics")]
fn blocked_preparation_factory(
    entered: Arc<Notify>,
    barrier: Arc<Barrier>,
) -> impl Fn() -> Result<WireframeApp, crate::WireframeError> + Clone {
    let handler: Handler<Envelope> = Arc::new(|_: &Envelope| Box::pin(async {}));
    move || {
        WireframeApp::new()?
            .route(1, Arc::clone(&handler))?
            .wrap(PreparationBarrier {
                entered: Arc::clone(&entered),
                barrier: Arc::clone(&barrier),
            })
    }
}

/// Interrupted preparation records a bounded cancelled startup duration.
#[cfg(feature = "metrics")]
#[test]
fn cancelled_preparation_records_a_bounded_startup_metric() {
    let recorder = DebuggingRecorder::new();
    let snapshotter = recorder.snapshotter();
    let result = metrics::with_local_recorder(&recorder, || {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("test runtime should build");
        runtime.block_on(async {
            let entered = Arc::new(Notify::new());
            let barrier = Arc::new(Barrier::new(2));
            let factory = blocked_preparation_factory(Arc::clone(&entered), Arc::clone(&barrier));
            let (shutdown_tx, shutdown_rx) = oneshot::channel();
            tokio::spawn(async move {
                entered.notified().await;
                let _ = shutdown_tx.send(());
            });
            prepare_application_or_shutdown(
                factory,
                async {
                    let _ = shutdown_rx.await;
                },
                Instant::now(),
            )
            .await
        })
    });

    assert!(matches!(result, Ok(None)));
    assert_startup_duration_outcome(&snapshotter, "cancelled");
}

/// Preparation failures record their bounded startup-duration outcome.
#[cfg(feature = "metrics")]
#[test]
fn preparation_failure_records_a_bounded_startup_metric() {
    let recorder = DebuggingRecorder::new();
    let snapshotter = recorder.snapshotter();
    let result = metrics::with_local_recorder(&recorder, || {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("test runtime should build");
        runtime.block_on(prepare_or_shutdown(
            async {
                Err::<(), _>(PrepareError::MiddlewareTransform {
                    source: Box::new(io::Error::other("preparation failed")),
                })
            },
            std::future::pending(),
            Instant::now(),
        ))
    });

    assert!(matches!(result, Err(ServerError::Prepare(_))));
    assert_startup_duration_outcome(&snapshotter, "preparation");
}

/// Successful startup records its bounded duration after workers are installed.
#[cfg(feature = "metrics")]
#[test]
fn successful_startup_records_a_bounded_startup_metric() {
    let recorder = DebuggingRecorder::new();
    let snapshotter = recorder.snapshotter();
    let result = metrics::with_local_recorder(&recorder, || {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("test runtime should build");
        runtime.block_on(start_default_server_and_stop_when_ready())
    });

    assert!(result.is_ok(), "server startup failed: {result:?}");
    assert_startup_duration_outcome(&snapshotter, "success");
}

#[cfg(feature = "metrics")]
async fn start_default_server_and_stop_when_ready()
-> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let server = WireframeServer::new(|| -> WireframeApp { WireframeApp::default() })
        .bind_existing_listener(free_listener()?)?;
    let (ready_tx, ready_rx) = oneshot::channel();
    let (shutdown_tx, shutdown_rx) = oneshot::channel();
    let server_task = tokio::spawn(async move {
        server
            .ready_signal(ready_tx)
            .run_with_shutdown(wait_for_shutdown_signal(shutdown_rx))
            .await
    });

    ready_rx.await?;
    shutdown_tx
        .send(())
        .map_err(|()| io::Error::other("server stopped before shutdown was signalled"))?;
    server_task.await??;
    Ok(())
}

#[cfg(feature = "metrics")]
async fn wait_for_shutdown_signal(shutdown_rx: oneshot::Receiver<()>) { let _ = shutdown_rx.await; }
