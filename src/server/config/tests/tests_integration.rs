//! Integration-style tests for server builder composition.

use std::{
    io,
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    },
};

use bincode::error::DecodeError;
use rstest::rstest;

use crate::{
    app::WireframeApp,
    server::{
        WireframeServer,
        test_util::{TestPreamble, factory, free_listener},
    },
};

#[rstest]
#[tokio::test]
async fn test_method_chaining(
    factory: impl Fn() -> WireframeApp + Send + Sync + Clone + 'static,
    free_listener: std::io::Result<std::net::TcpListener>,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let listener = free_listener?;
    let handler_invoked = Arc::new(AtomicUsize::new(0));
    let counter = handler_invoked.clone();
    let server = WireframeServer::new(factory)
        .workers(2)
        .with_preamble::<TestPreamble>()
        .on_preamble_decode_success(move |_p: &TestPreamble, _| {
            let c = counter.clone();
            Box::pin(async move {
                c.fetch_add(1, Ordering::SeqCst);
                Ok(())
            })
        })
        .on_preamble_decode_failure(|_: &DecodeError, _| {
            Box::pin(async { Ok::<(), io::Error>(()) })
        })
        .bind_existing_listener(listener)
        .expect("Failed to bind");
    assert_eq!(
        server.worker_count(),
        2,
        "server.worker_count() should equal 2"
    );
    assert!(
        server.local_addr().is_some(),
        "assertion failed: server.local_addr().is_some()"
    );
    assert_eq!(
        handler_invoked.load(Ordering::SeqCst),
        0,
        "handler_invoked.load(Ordering::SeqCst) should equal 0"
    );
    Ok(())
}

#[rstest]
#[tokio::test]
async fn test_server_configuration_persistence(
    factory: impl Fn() -> WireframeApp + Send + Sync + Clone + 'static,
    free_listener: std::io::Result<std::net::TcpListener>,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let listener = free_listener?;
    let server = WireframeServer::new(factory)
        .workers(5)
        .bind_existing_listener(listener)
        .expect("Failed to bind");
    assert_eq!(
        server.worker_count(),
        5,
        "server.worker_count() should equal 5"
    );
    assert!(
        server.local_addr().is_some(),
        "assertion failed: server.local_addr().is_some()"
    );
    Ok(())
}

#[rstest]
fn test_extreme_worker_counts(factory: impl Fn() -> WireframeApp + Send + Sync + Clone + 'static) {
    let server = WireframeServer::new(factory);
    let max_worker_server = server.workers(usize::MAX);
    assert_eq!(
        max_worker_server.worker_count(),
        usize::MAX,
        "server.worker_count() should equal usize::MAX"
    );
    assert_eq!(
        max_worker_server.workers(0).worker_count(),
        1,
        "server.workers(0).worker_count() should equal 1"
    );
}
