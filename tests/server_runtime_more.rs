//! Additional runtime tests for `WireframeServer`.
use std::{
    net::{Ipv4Addr, SocketAddr},
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    },
};

use bincode::error::DecodeError;
use rstest::rstest;
use tokio::{
    net::TcpStream,
    sync::oneshot,
    time::{Duration, timeout},
};
use tracing_test::traced_test;
use wireframe::{app::WireframeApp, server::WireframeServer};
use wireframe_testing::{LoggerHandle, logger};

use crate::server_helpers::{TestPreamble, factory, free_port};

mod server_helpers;

#[rstest]
#[tokio::test]
async fn test_server_graceful_shutdown_with_ctrl_c_simulation(
    factory: impl Fn() -> WireframeApp + Send + Sync + Clone + 'static,
    free_port: SocketAddr,
) {
    let server = WireframeServer::new(factory)
        .workers(2)
        .bind(free_port)
        .expect("Failed to bind");

    let shutdown_future = async {
        tokio::time::sleep(Duration::from_millis(50)).await;
    };

    let start = std::time::Instant::now();
    let result = timeout(
        Duration::from_millis(1000),
        server.run_with_shutdown(shutdown_future),
    )
    .await;
    let elapsed = start.elapsed();

    assert!(result.is_ok());
    assert!(result.unwrap().is_ok());
    assert!(elapsed < Duration::from_millis(500));
}

#[rstest]
#[tokio::test]
async fn test_multiple_worker_creation(
    factory: impl Fn() -> WireframeApp + Send + Sync + Clone + 'static,
    free_port: SocketAddr,
) {
    let _ = &factory;
    let call_count = Arc::new(AtomicUsize::new(0));
    let call_count_clone = call_count.clone();

    let factory = move || {
        call_count_clone.fetch_add(1, Ordering::SeqCst);
        WireframeApp::default()
    };

    let server = WireframeServer::new(factory)
        .workers(3)
        .bind(free_port)
        .expect("Failed to bind");

    let shutdown_future = async {
        tokio::time::sleep(Duration::from_millis(10)).await;
    };

    let result = timeout(
        Duration::from_millis(1000),
        server.run_with_shutdown(shutdown_future),
    )
    .await;

    assert!(result.is_ok());
    assert!(result.unwrap().is_ok());
    // no connections handled, factory should not be called
    assert_eq!(call_count.load(Ordering::SeqCst), 0);
}

#[rstest]
#[tokio::test]
async fn test_server_configuration_persistence(
    factory: impl Fn() -> WireframeApp + Send + Sync + Clone + 'static,
    free_port: SocketAddr,
) {
    let server = WireframeServer::new(factory).workers(5);

    assert_eq!(server.worker_count(), 5);

    let server = server.bind(free_port).expect("Failed to bind");
    assert_eq!(server.worker_count(), 5);
    assert!(server.local_addr().is_some());
}

#[rstest]
fn test_preamble_callbacks_reset_on_type_change(
    factory: impl Fn() -> WireframeApp + Send + Sync + Clone + 'static,
) {
    let server = WireframeServer::new(factory)
        .on_preamble_decode_success(|&(), _| Box::pin(async { Ok(()) }))
        .on_preamble_decode_failure(|_: &DecodeError| {});

    assert!(server.has_preamble_success());
    assert!(server.has_preamble_failure());

    let server = server.with_preamble::<TestPreamble>();
    assert!(!server.has_preamble_success());
    assert!(!server.has_preamble_failure());
}

#[rstest]
fn test_extreme_worker_counts(factory: impl Fn() -> WireframeApp + Send + Sync + Clone + 'static) {
    let server = WireframeServer::new(factory);

    let server = server.workers(usize::MAX);
    assert_eq!(server.worker_count(), usize::MAX);

    let server = server.workers(0);
    assert_eq!(server.worker_count(), 1);
}

#[rstest]
#[tokio::test]
async fn test_bind_to_multiple_addresses(
    factory: impl Fn() -> WireframeApp + Send + Sync + Clone + 'static,
    free_port: SocketAddr,
) {
    let server = WireframeServer::new(factory);
    let addr1 = free_port;
    let addr2 = {
        let addr = SocketAddr::new(Ipv4Addr::LOCALHOST.into(), 0);
        let listener = std::net::TcpListener::bind(addr).unwrap();
        listener.local_addr().unwrap()
    };

    let server = server.bind(addr1).expect("Failed to bind first address");
    let first_local_addr = server.local_addr().unwrap();

    let server = server.bind(addr2).expect("Failed to bind second address");
    let second_local_addr = server.local_addr().unwrap();

    assert_ne!(first_local_addr.port(), second_local_addr.port());
    assert_eq!(second_local_addr.ip(), addr2.ip());
}

#[test]
fn test_server_debug_compilation_guard() {
    assert!(cfg!(debug_assertions));
}

/// Ensure the server survives panicking connection tasks.
///
/// The test spawns a server with a connection setup callback that
/// immediately panics. Logs are captured so the panic message and peer
/// address can be asserted. A first client
/// connection triggers the panic and writes dummy preamble bytes to ensure
/// the panic is logged. The client's peer address is captured before
/// dropping the connection so the error log can be validated. A second
/// connection verifies the server continues accepting new clients after the
/// failure. Finally, the logs are scanned for the expected error entry
#[rstest]
#[traced_test]
#[tokio::test]
async fn connection_panic_is_caught(
    factory: impl Fn() -> WireframeApp + Send + Sync + Clone + 'static,
    mut logger: LoggerHandle,
) {
    let app_factory = move || {
        factory()
            .on_connection_setup(|| async { panic!("boom") })
            .unwrap()
    };
    let server = WireframeServer::new(app_factory)
        .workers(1)
        .bind("127.0.0.1:0".parse().unwrap())
        .expect("bind");
    let addr = server.local_addr().unwrap();

    let (tx, rx) = oneshot::channel();
    let handle = tokio::spawn(async move {
        server
            .run_with_shutdown(async {
                let _ = rx.await;
            })
            .await
            .unwrap();
    });

    let first = TcpStream::connect(addr)
        .await
        .expect("first connection should succeed");
    first.writable().await.unwrap();
    first.try_write(&[0; 8]).unwrap();
    drop(first);
    TcpStream::connect(addr)
        .await
        .expect("second connection should succeed after panic");

    let _ = tx.send(());
    handle.await.unwrap();

    let mut found_task = false;
    let mut found_msg = false;
    let mut found_addr = false;
    while let Some(record) = logger.pop() {
        if record.args().contains("connection task panicked") {
            found_task = true;
        }
        if record.args().contains("boom") {
            found_msg = true;
        }
        if record.args().contains("peer_addr") {
            found_addr = true;
        }
    }
    assert!(found_task);
    assert!(found_msg);
    assert!(found_addr);
}
