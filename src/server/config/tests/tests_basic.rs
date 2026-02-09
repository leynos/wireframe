//! Basic configuration tests for `WireframeServer`.

use rstest::rstest;

use crate::{
    app::WireframeApp,
    server::{
        WireframeServer,
        default_worker_count,
        test_util::{bind_server, factory, free_listener, listener_addr},
    },
};

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
    assert_eq!(server.worker_count(), default_worker_count());
}

#[rstest]
fn test_workers_configuration(factory: impl Fn() -> WireframeApp + Send + Sync + Clone + 'static) {
    let server = WireframeServer::new(factory);
    let server = server.workers(4);
    assert_eq!(server.worker_count(), 4);
    let server = server.workers(100);
    assert_eq!(server.worker_count(), 100);
    assert_eq!(server.workers(0).worker_count(), 1);
}

#[rstest]
fn test_local_addr_before_bind(factory: impl Fn() -> WireframeApp + Send + Sync + Clone + 'static) {
    assert!(WireframeServer::new(factory).local_addr().is_none());
}

#[rstest]
#[tokio::test]
async fn test_local_addr_after_bind(
    factory: impl Fn() -> WireframeApp + Send + Sync + Clone + 'static,
    free_listener: std::net::TcpListener,
) {
    let expected = listener_addr(&free_listener);
    let local_addr = bind_server(factory, free_listener)
        .local_addr()
        .expect("local address missing");
    assert_eq!(local_addr, expected);
}
