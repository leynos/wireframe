//! Coverage for connection task spawning and panic recovery behaviour.

use rstest::rstest;
use tokio::net::{TcpListener, TcpStream};
use tokio_util::task::TaskTracker;
use tracing_test::traced_test;

use super::*;
use crate::{app::WireframeApp, server::test_util::factory};

/// Panics in connection handlers are logged and do not tear down the worker.
#[rstest]
#[traced_test]
#[tokio::test]
async fn spawn_connection_task_logs_panic(
    factory: impl Fn() -> WireframeApp + Send + Sync + Clone + 'static,
) {
    let app = Arc::new(
        factory()
            .on_connection_setup(|| async { panic!("boom") })
            .expect("failed to install panic setup callback")
            .prepare()
            .await
            .expect("prepare app"),
    );
    let tracker = TaskTracker::new();
    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind listener");
    let addr = listener.local_addr().expect("listener.local_addr");

    let handle = tokio::spawn({
        let task_tracker = tracker.clone();
        let app = Arc::clone(&app);
        async move {
            let (stream, _) = listener.accept().await.expect("accept");
            spawn_connection_task(stream, app, PreambleHooks::<()>::default(), &task_tracker);
            task_tracker.close();
            task_tracker.wait().await;
        }
    });

    let client = TcpStream::connect(addr).await.expect("connect");
    let peer_addr = client.local_addr().expect("client.local_addr");
    client.writable().await.expect("client.writable");
    client.try_write(&[0; 8]).expect("client.try_write");
    drop(client);

    handle.await.expect("join connection task driver");
    tokio::task::yield_now().await;

    logs_assert(|lines: &[&str]| {
        lines
            .iter()
            .find(|line| {
                line.contains("connection task panicked")
                    && line.contains("panic=boom")
                    && line.contains(&format!("peer_addr=Some({peer_addr})"))
            })
            .map(|_| ())
            .ok_or_else(|| "panic log not found".to_string())
    });
}

/// Non-string panic payloads are formatted with `Debug` in logs.
#[rstest]
#[traced_test]
#[tokio::test]
async fn spawn_connection_task_logs_non_string_panic(
    factory: impl Fn() -> WireframeApp + Send + Sync + Clone + 'static,
) {
    let app = Arc::new(
        factory()
            .on_connection_setup(|| async { std::panic::panic_any(5_u32) })
            .expect("install panic setup callback")
            .prepare()
            .await
            .expect("prepare app"),
    );
    let tracker = TaskTracker::new();
    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind listener");
    let addr = listener.local_addr().expect("listener.local_addr");

    let handle = tokio::spawn({
        let task_tracker = tracker.clone();
        let app = Arc::clone(&app);
        async move {
            let (stream, _) = listener.accept().await.expect("accept");
            spawn_connection_task(stream, app, PreambleHooks::<()>::default(), &task_tracker);
            task_tracker.close();
            task_tracker.wait().await;
        }
    });

    let client = TcpStream::connect(addr).await.expect("connect");
    let peer_addr = client.local_addr().expect("client.local_addr");
    client.writable().await.expect("client.writable");
    client.try_write(&[0; 8]).expect("client.try_write");
    drop(client);

    handle.await.expect("join connection task driver");
    tokio::task::yield_now().await;

    logs_assert(|lines: &[&str]| {
        lines
            .iter()
            .find(|line| {
                line.contains("connection task panicked")
                    && line.contains("panic=Any")
                    && line.contains(&format!("peer_addr=Some({peer_addr})"))
            })
            .map(|_| ())
            .ok_or_else(|| "panic log not found".to_string())
    });
}
