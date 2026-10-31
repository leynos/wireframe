//! Regression coverage for panic-aware supervisor cleanup.

use tokio::sync::oneshot;
use tokio_util::{sync::CancellationToken, task::TaskTracker};

use super::run_with_panic_cleanup;
use crate::server::ServerError;

async fn panicking_supervisor() -> Result<(), ServerError> {
    tokio::task::yield_now().await;
    panic!("supervisor cleanup test panic");
}

#[tokio::test]
async fn panic_cancels_and_drains_tracked_workers_before_resuming() {
    let token = CancellationToken::new();
    let tracker = TaskTracker::new();
    let (worker_done_tx, worker_done_rx) = oneshot::channel();
    let worker_token = token.clone();
    tracker.spawn(async move {
        worker_token.cancelled().await;
        let _ = worker_done_tx.send(());
    });

    let supervisor_token = token.clone();
    let supervisor = tokio::spawn(async move {
        run_with_panic_cleanup(panicking_supervisor(), &supervisor_token, &tracker).await
    });

    assert!(
        supervisor
            .await
            .expect_err("supervisor should panic")
            .is_panic()
    );
    worker_done_rx
        .await
        .expect("panic cleanup should drain the tracked worker");
    assert!(token.is_cancelled());
}
