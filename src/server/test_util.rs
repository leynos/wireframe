//! Test helpers shared across server modules.

use std::net::{Ipv4Addr, SocketAddr, TcpListener as StdTcpListener};

use bincode::{Decode, Encode};
use rstest::fixture;

use super::WireframeServer;
use crate::app::WireframeApp;

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
/// Returns a bound listener on a free port.
///
/// Keeping the listener alive reserves the port, avoiding races where another
/// process might claim it before the test is ready.
pub fn free_listener() -> StdTcpListener {
    let addr = SocketAddr::new(Ipv4Addr::LOCALHOST.into(), 0);
    StdTcpListener::bind(addr).expect("failed to bind free port listener")
}

pub fn bind_server<F>(factory: F, std_listener: StdTcpListener) -> WireframeServer<F, ()>
where
    F: Fn() -> WireframeApp + Send + Sync + Clone + 'static,
{
    WireframeServer::new(factory)
        .bind_listener(std_listener)
        .expect("Failed to bind")
}

pub fn server_with_preamble<F>(factory: F) -> WireframeServer<F, TestPreamble>
where
    F: Fn() -> WireframeApp + Send + Sync + Clone + 'static,
{
    WireframeServer::new(factory).with_preamble::<TestPreamble>()
}
