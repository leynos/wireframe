//! Tests for server runtime behaviour.

use std::{
    io,
    sync::{
        Arc,
        Mutex,
        atomic::{AtomicUsize, Ordering},
    },
};

use rstest::rstest;
use tokio::{
    net::TcpListener,
    sync::oneshot,
    task::yield_now,
    time::{Duration, Instant, advance, timeout},
};
use tokio_util::{sync::CancellationToken, task::TaskTracker};

use super::{
    AcceptLoopOptions,
    BackoffConfig,
    MockAcceptListener,
    PreambleHooks,
    WireframeServer,
    accept_loop,
};
use crate::{
    app::WireframeApp,
    server::test_util::{bind_server, factory, free_listener},
};

#[rstest]
#[tokio::test]
async fn test_run_with_immediate_shutdown(
    factory: impl Fn() -> WireframeApp + Send + Sync + Clone + 'static,
    free_listener: std::net::TcpListener,
) {
    let server = bind_server(factory, free_listener);
    let shutdown_future = async { tokio::time::sleep(Duration::from_millis(10)).await };
    let result = timeout(
        Duration::from_millis(1000),
        server.run_with_shutdown(shutdown_future),
    )
    .await;
    assert!(result.is_ok());
    assert!(result.expect("server did not finish in time").is_ok());
}

#[rstest]
#[tokio::test]
async fn test_server_graceful_shutdown_with_ctrl_c_simulation(
    factory: impl Fn() -> WireframeApp + Send + Sync + Clone + 'static,
    free_listener: std::net::TcpListener,
) {
    let server = bind_server(factory, free_listener);
    let (tx, rx) = oneshot::channel();
    let handle = tokio::spawn(async move {
        server
            .run_with_shutdown(async {
                let _ = rx.await;
            })
            .await
            .expect("server run failed");
    });
    let _ = tx.send(());
    handle.await.expect("server join error");
}

#[rstest]
#[tokio::test]
async fn test_multiple_worker_creation(free_listener: std::net::TcpListener) {
    let call_count = Arc::new(AtomicUsize::new(0));
    let clone = call_count.clone();
    let factory = move || -> WireframeApp {
        clone.fetch_add(1, Ordering::SeqCst);
        WireframeApp::default()
    };
    let server = WireframeServer::new(factory)
        .workers(3)
        .bind_existing_listener(free_listener)
        .expect("Failed to bind");
    let shutdown_future = async { tokio::time::sleep(Duration::from_millis(10)).await };
    let result = timeout(
        Duration::from_millis(1000),
        server.run_with_shutdown(shutdown_future),
    )
    .await;
    assert!(result.is_ok());
    assert!(result.expect("server did not finish in time").is_ok());
}

#[rstest]
#[tokio::test]
async fn test_accept_loop_shutdown_signal(
    factory: impl Fn() -> WireframeApp + Send + Sync + Clone + 'static,
) {
    let token = CancellationToken::new();
    let tracker = TaskTracker::new();
    let listener = Arc::new(
        TcpListener::bind("127.0.0.1:0")
            .await
            .expect("failed to bind test listener"),
    );

    tracker.spawn(accept_loop(
        listener,
        factory,
        AcceptLoopOptions::<()> {
            preamble: PreambleHooks::default(),
            shutdown: token.clone(),
            tracker: tracker.clone(),
            backoff: BackoffConfig::default(),
        },
    ));

    token.cancel();
    tracker.close();

    let result = timeout(Duration::from_millis(100), tracker.wait()).await;
    assert!(result.is_ok());
}

/// Creates a mock listener that fails with exponential backoff tracking.
fn setup_backoff_mock_listener(
    calls: &Arc<Mutex<Vec<Instant>>>,
    num_calls: usize,
) -> MockAcceptListener {
    let mut listener = MockAcceptListener::new();
    let call_log = Arc::clone(calls);
    listener
        .expect_accept()
        .returning(move || {
            let call_log = Arc::clone(&call_log);
            Box::pin(async move {
                call_log.lock().expect("lock").push(Instant::now());
                Err(io::Error::other("mock error"))
            })
        })
        .times(num_calls);
    listener
        .expect_local_addr()
        .returning(|| Ok("127.0.0.1:0".parse().expect("addr parse")))
        .times(num_calls);
    listener
}

/// Validates that recorded call intervals match expected backoff delays.
fn assert_backoff_intervals(
    calls: &[Instant],
    expected: &[Duration],
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let intervals: Vec<_> = calls
        .windows(2)
        .map(|window| {
            let a = window
                .first()
                .ok_or_else(|| "window has first element".to_string())?;
            let b = window
                .get(1)
                .ok_or_else(|| "window has second element".to_string())?;
            b.checked_duration_since(*a)
                .ok_or_else(|| "instants should be monotonically increasing".to_string())
        })
        .collect::<Result<_, _>>()?;

    if intervals.len() != expected.len() {
        return Err(format!(
            "interval count mismatch: got {}, expected {}",
            intervals.len(),
            expected.len()
        )
        .into());
    }

    for (interval, expected) in intervals.into_iter().zip(expected.iter()) {
        if interval != *expected {
            return Err(
                format!("interval mismatch: got {interval:?}, expected {expected:?}").into(),
            );
        }
    }

    Ok(())
}

#[rstest]
#[tokio::test(start_paused = true)]
async fn test_accept_loop_exponential_backoff_async(
    factory: impl Fn() -> WireframeApp + Send + Sync + Clone + 'static,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let calls = Arc::new(Mutex::new(Vec::new()));
    let listener = Arc::new(setup_backoff_mock_listener(&calls, 4));
    let token = CancellationToken::new();
    let tracker = TaskTracker::new();
    let backoff = BackoffConfig {
        initial_delay: Duration::from_millis(5),
        max_delay: Duration::from_millis(20),
    };

    tracker.spawn(accept_loop(
        listener,
        factory,
        AcceptLoopOptions::<()> {
            preamble: PreambleHooks::default(),
            shutdown: token.clone(),
            tracker: tracker.clone(),
            backoff,
        },
    ));

    yield_now().await;

    let first_call = {
        let calls = calls.lock().expect("lock");
        assert_eq!(calls.len(), 1);
        calls.first().copied().expect("call record missing")
    };

    for ms in [5, 10, 20] {
        advance(Duration::from_millis(ms)).await;
        yield_now().await;
    }

    token.cancel();
    advance(Duration::from_millis(20)).await;
    yield_now().await;
    tracker.close();
    tracker.wait().await;

    let calls = calls.lock().expect("lock");
    assert_eq!(calls.len(), 4);
    let first = calls.first().copied().expect("at least one call logged");
    assert_eq!(first, first_call);
    let expected = [
        Duration::from_millis(5),
        Duration::from_millis(10),
        Duration::from_millis(20),
    ];
    assert_backoff_intervals(&calls, &expected)?;
    Ok(())
}
