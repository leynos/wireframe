//! Startup cancellation coverage for the server runtime.

#[cfg(feature = "metrics")]
use std::io;
use std::sync::Arc;

use async_trait::async_trait;
#[cfg(feature = "metrics")]
use metrics_util::debugging::{DebugValue, DebuggingRecorder};
use tokio::{
    sync::{Barrier, Notify, oneshot},
    time::{Duration, timeout},
};

use super::WireframeServer;
use crate::{
    app::{Envelope, Handler, WireframeApp},
    middleware::{HandlerService, Transform},
    server::test_util::free_listener,
};
#[cfg(feature = "metrics")]
use crate::{metrics::SERVER_STARTUP_FAILURES, server::ServerError};

/// Middleware that holds application preparation until the test releases it.
struct PreparationBarrier {
    /// Announces that preparation reached the blocking transform.
    entered: Arc<Notify>,
    /// Coordinates preparation with the test's shutdown signal.
    barrier: Arc<Barrier>,
}

#[async_trait]
impl Transform<HandlerService<Envelope>> for PreparationBarrier {
    type Output = HandlerService<Envelope>;

    /// Wait until the test lets the preparation transform continue.
    async fn transform(&self, service: HandlerService<Envelope>) -> Self::Output {
        self.entered.notify_one();
        self.barrier.wait().await;
        service
    }
}

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
    let metrics = snapshotter.snapshot().into_vec();
    assert!(metrics.iter().any(|(key, _, _, value)| {
        key.key().name() == SERVER_STARTUP_FAILURES
            && key
                .key()
                .labels()
                .any(|label| label.key() == "stage" && label.value() == "factory_build")
            && matches!(value, DebugValue::Counter(count) if *count == 1)
    }));
}
