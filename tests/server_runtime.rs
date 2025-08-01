//! Runtime behaviour tests for `WireframeServer`.
use std::{
    net::{Ipv4Addr, SocketAddr},
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    },
};

use bincode::error::DecodeError;
use rstest::rstest;
use tokio::time::{Duration, timeout};
use wireframe::{app::WireframeApp, server::WireframeServer};

use crate::server_helpers::{TestPreamble, bind_server, factory, free_port, server_with_preamble};

mod server_helpers;

#[rstest]
fn test_new_server_creation(factory: impl Fn() -> WireframeApp + Send + Sync + Clone + 'static) {
    let server = WireframeServer::new(factory);
    assert!(server.worker_count() >= 1);
    assert!(server.local_addr().is_none());
}

#[rstest]
fn test_new_server_default_worker_count(
    factory: impl Fn() -> WireframeApp + Send + Sync + Clone + 'static,
) {
    let server = WireframeServer::new(factory);
    let expected_workers = std::thread::available_parallelism()
        .map_or(1, std::num::NonZeroUsize::get)
        .max(1);
    assert_eq!(server.worker_count(), expected_workers);
}

#[rstest]
fn test_workers_configuration(factory: impl Fn() -> WireframeApp + Send + Sync + Clone + 'static) {
    let server = WireframeServer::new(factory);

    let server = server.workers(4);
    assert_eq!(server.worker_count(), 4);

    let server = server.workers(100);
    assert_eq!(server.worker_count(), 100);

    let server = server.workers(0);
    assert_eq!(server.worker_count(), 1);
}

#[rstest]
fn test_with_preamble_type_conversion(
    factory: impl Fn() -> WireframeApp + Send + Sync + Clone + 'static,
) {
    let server = WireframeServer::new(factory);
    let server_with_preamble = server.with_preamble::<TestPreamble>();
    assert_eq!(
        server_with_preamble.worker_count(),
        std::thread::available_parallelism()
            .map_or(1, std::num::NonZeroUsize::get)
            .max(1)
    );
}

#[rstest]
#[tokio::test]
async fn test_bind_success(
    factory: impl Fn() -> WireframeApp + Send + Sync + Clone + 'static,
    free_port: SocketAddr,
) {
    let server = bind_server(factory, free_port);
    let bound_addr = server.local_addr().unwrap();
    assert_eq!(bound_addr.ip(), free_port.ip());
}

#[rstest]
#[tokio::test]
async fn test_bind_invalid_address(
    factory: impl Fn() -> WireframeApp + Send + Sync + Clone + 'static,
) {
    let server = WireframeServer::new(factory);
    // Binding to a privileged port typically requires elevated privileges.
    let addr = SocketAddr::new(Ipv4Addr::LOCALHOST.into(), 80);
    let result = server.bind(addr);
    if !cfg!(target_os = "windows") && std::env::var("USER").is_ok_and(|u| u != "root") {
        assert!(result.is_err(), "Expected bind to privileged port to fail");
    }
}

#[rstest]
fn test_local_addr_before_bind(factory: impl Fn() -> WireframeApp + Send + Sync + Clone + 'static) {
    let server = WireframeServer::new(factory);
    assert!(server.local_addr().is_none());
}

#[rstest]
#[tokio::test]
async fn test_local_addr_after_bind(
    factory: impl Fn() -> WireframeApp + Send + Sync + Clone + 'static,
    free_port: SocketAddr,
) {
    let server = bind_server(factory, free_port);
    let local_addr = server.local_addr();
    assert!(local_addr.is_some());
    assert_eq!(local_addr.unwrap().ip(), free_port.ip());
}

#[rstest]
#[tokio::test]
async fn test_preamble_success_callback(
    factory: impl Fn() -> WireframeApp + Send + Sync + Clone + 'static,
) {
    let callback_counter = Arc::new(AtomicUsize::new(0));
    let counter_clone = callback_counter.clone();

    let server = server_with_preamble(factory).on_preamble_decode_success(
        move |_preamble: &TestPreamble, _| {
            let cnt = counter_clone.clone();
            Box::pin(async move {
                cnt.fetch_add(1, Ordering::SeqCst);
                Ok(())
            })
        },
    );

    assert_eq!(callback_counter.load(Ordering::SeqCst), 0);
    assert!(server.has_preamble_success());
}

#[rstest]
#[tokio::test]
async fn test_preamble_failure_callback(
    factory: impl Fn() -> WireframeApp + Send + Sync + Clone + 'static,
) {
    let callback_counter = Arc::new(AtomicUsize::new(0));
    let counter_clone = callback_counter.clone();

    let server =
        server_with_preamble(factory).on_preamble_decode_failure(move |_error: &DecodeError| {
            counter_clone.fetch_add(1, Ordering::SeqCst);
        });

    assert_eq!(callback_counter.load(Ordering::SeqCst), 0);
    assert!(server.has_preamble_failure());
}

#[rstest]
#[tokio::test]
async fn test_method_chaining(
    factory: impl Fn() -> WireframeApp + Send + Sync + Clone + 'static,
    free_port: SocketAddr,
) {
    let callback_invoked = Arc::new(AtomicUsize::new(0));
    let counter_clone = callback_invoked.clone();

    let server = WireframeServer::new(factory)
        .workers(2)
        .with_preamble::<TestPreamble>()
        .on_preamble_decode_success(move |_: &TestPreamble, _| {
            let cnt = counter_clone.clone();
            Box::pin(async move {
                cnt.fetch_add(1, Ordering::SeqCst);
                Ok(())
            })
        })
        .on_preamble_decode_failure(|_: &DecodeError| {
            tracing::error!("Preamble decode failed");
        })
        .bind(free_port)
        .expect("Failed to bind");

    assert_eq!(server.worker_count(), 2);
    assert!(server.local_addr().is_some());
    assert!(server.has_preamble_success());
    assert!(server.has_preamble_failure());
}

#[rstest]
#[tokio::test]
#[should_panic(expected = "`bind` must be called before `run`")]
async fn test_run_without_bind_panics(
    factory: impl Fn() -> WireframeApp + Send + Sync + Clone + 'static,
) {
    let server = WireframeServer::new(factory);
    let _ = timeout(Duration::from_millis(100), server.run()).await;
}

#[rstest]
#[tokio::test]
#[should_panic(expected = "`bind` must be called before `run`")]
async fn test_run_with_shutdown_without_bind_panics(
    factory: impl Fn() -> WireframeApp + Send + Sync + Clone + 'static,
) {
    let server = WireframeServer::new(factory);
    let shutdown_future = async { tokio::time::sleep(Duration::from_millis(10)).await };
    let _ = timeout(
        Duration::from_millis(100),
        server.run_with_shutdown(shutdown_future),
    )
    .await;
}

#[rstest]
#[tokio::test]
async fn test_run_with_immediate_shutdown(
    factory: impl Fn() -> WireframeApp + Send + Sync + Clone + 'static,
    free_port: SocketAddr,
) {
    let server = WireframeServer::new(factory)
        .workers(1)
        .bind(free_port)
        .expect("Failed to bind");

    let shutdown_future = async {};

    let result = timeout(
        Duration::from_millis(1000),
        server.run_with_shutdown(shutdown_future),
    )
    .await;

    assert!(result.is_ok());
    assert!(result.unwrap().is_ok());
}
