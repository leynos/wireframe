//! Binding behaviour tests for `WireframeServer`.

use rstest::rstest;

use crate::{
    app::WireframeApp,
    server::{
        WireframeServer,
        test_util::{factory, free_listener, listener_addr},
    },
};

#[rstest]
#[tokio::test]
async fn test_bind_success(
    factory: impl Fn() -> WireframeApp + Send + Sync + Clone + 'static,
    free_listener: std::net::TcpListener,
) {
    let expected = listener_addr(&free_listener);
    let local_addr = WireframeServer::new(factory)
        .bind_existing_listener(free_listener)
        .expect("Failed to bind")
        .local_addr()
        .expect("local address missing");
    assert_eq!(local_addr, expected);
}

#[rstest]
#[tokio::test]
async fn test_bind_to_multiple_addresses(
    factory: impl Fn() -> WireframeApp + Send + Sync + Clone + 'static,
    free_listener: std::net::TcpListener,
) {
    let addr1 = listener_addr(&free_listener);

    let server = WireframeServer::new(factory);
    let server = server
        .bind_existing_listener(free_listener)
        .expect("Failed to bind first address");
    let first = server.local_addr().expect("first bound address missing");
    assert_eq!(first, addr1);

    let server = server
        .bind(std::net::SocketAddr::new(addr1.ip(), 0))
        .expect("Failed to bind second address");
    let second = server.local_addr().expect("second bound address missing");
    assert_eq!(second.ip(), addr1.ip());
    assert_ne!(first.port(), second.port());
}
