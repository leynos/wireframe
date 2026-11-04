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
    free_listener: std::io::Result<std::net::TcpListener>,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let listener = free_listener?;
    let expected = listener_addr(&listener)?;
    let local_addr = WireframeServer::new(factory)
        .bind_existing_listener(listener)
        .expect("Failed to bind")
        .local_addr()
        .expect("local address missing");
    assert_eq!(local_addr, expected, "local_addr should equal expected");
    Ok(())
}

#[rstest]
#[tokio::test]
async fn test_bind_to_multiple_addresses(
    factory: impl Fn() -> WireframeApp + Send + Sync + Clone + 'static,
    free_listener: std::io::Result<std::net::TcpListener>,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let listener = free_listener?;
    let addr1 = listener_addr(&listener)?;

    let server = WireframeServer::new(factory);
    let first_server = server
        .bind_existing_listener(listener)
        .expect("Failed to bind first address");
    let first = first_server
        .local_addr()
        .expect("first bound address missing");
    assert_eq!(first, addr1, "first should equal addr1");

    let second_server = first_server
        .bind(std::net::SocketAddr::new(addr1.ip(), 0))
        .expect("Failed to bind second address");
    let second = second_server
        .local_addr()
        .expect("second bound address missing");
    assert_eq!(
        second.ip(),
        addr1.ip(),
        "second.ip() should equal addr1.ip()"
    );
    assert_ne!(
        first.port(),
        second.port(),
        "first.port() should not equal second.port()"
    );
    Ok(())
}
