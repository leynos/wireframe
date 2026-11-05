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
    assert!(
        server.worker_count() >= 1,
        "assertion failed: server.worker_count() >= 1"
    );
    assert!(
        server.local_addr().is_none(),
        "assertion failed: server.local_addr().is_none()"
    );
}

#[rstest]
fn test_new_server_default_worker_count(
    factory: impl Fn() -> WireframeApp + Send + Sync + Clone + 'static,
) {
    let server = WireframeServer::new(factory);
    assert_eq!(
        server.worker_count(),
        default_worker_count(),
        "server.worker_count() should equal default_worker_count()"
    );
}

#[rstest]
fn test_workers_configuration(factory: impl Fn() -> WireframeApp + Send + Sync + Clone + 'static) {
    let server = WireframeServer::new(factory);
    let four_worker_server = server.workers(4);
    assert_eq!(
        four_worker_server.worker_count(),
        4,
        "server.worker_count() should equal 4"
    );
    let hundred_worker_server = four_worker_server.workers(100);
    assert_eq!(
        hundred_worker_server.worker_count(),
        100,
        "server.worker_count() should equal 100"
    );
    assert_eq!(
        hundred_worker_server.workers(0).worker_count(),
        1,
        "server.workers(0).worker_count() should equal 1"
    );
}

#[rstest]
fn test_local_addr_before_bind(factory: impl Fn() -> WireframeApp + Send + Sync + Clone + 'static) {
    assert!(
        WireframeServer::new(factory).local_addr().is_none(),
        "assertion failed: WireframeServer::new(factory).local_addr().is_none()"
    );
}

#[rstest]
#[tokio::test]
async fn test_local_addr_after_bind(
    factory: impl Fn() -> WireframeApp + Send + Sync + Clone + 'static,
    free_listener: std::io::Result<std::net::TcpListener>,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let listener = free_listener?;
    let expected = listener_addr(&listener)?;
    let local_addr = bind_server(factory, listener)?
        .local_addr()
        .expect("local address missing");
    assert_eq!(local_addr, expected, "local_addr should equal expected");
    Ok(())
}
