//! Test helpers shared across server modules.

use std::net::{Ipv4Addr, SocketAddr};

use bincode::{Decode, Encode};
use rstest::fixture;

use super::{Bound, WireframeServer};
use crate::app::WireframeApp;

#[cfg_attr(
    not(test),
    expect(dead_code, reason = "Used in builder tests via fixtures")
)]
#[cfg_attr(test, allow(dead_code, reason = "Used in builder tests via fixtures"))]
#[derive(Debug, Clone, PartialEq, Encode, Decode)]
pub struct TestPreamble {
    pub id: u32,
    pub message: String,
}

#[fixture]
pub fn factory() -> impl Fn() -> WireframeApp + Send + Sync + Clone + 'static {
    || WireframeApp::default()
}

#[fixture]
pub fn free_port() -> SocketAddr {
    let addr = SocketAddr::new(Ipv4Addr::LOCALHOST.into(), 0);
    let listener = std::net::TcpListener::bind(addr).expect("failed to bind free port listener");
    listener
        .local_addr()
        .expect("failed to read free port listener address")
}

pub fn bind_server<F>(factory: F, addr: SocketAddr) -> WireframeServer<F, (), Bound>
where
    F: Fn() -> WireframeApp + Send + Sync + Clone + 'static,
{
    WireframeServer::new(factory)
        .bind(addr)
        .expect("Failed to bind")
}

#[cfg_attr(
    not(test),
    expect(dead_code, reason = "Only used in configuration tests")
)]
#[cfg_attr(test, allow(dead_code, reason = "Only used in configuration tests"))]
pub fn server_with_preamble<F>(factory: F) -> WireframeServer<F, TestPreamble>
where
    F: Fn() -> WireframeApp + Send + Sync + Clone + 'static,
{
    WireframeServer::new(factory).with_preamble::<TestPreamble>()
}
