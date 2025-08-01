//! Shared utilities for integration tests.
//!
//! Provides fixtures for a basic [`WireframeApp`] factory and an unused
//! local port. These helpers reduce duplication across test modules.

use std::net::{Ipv4Addr, SocketAddr, TcpListener};

use rstest::fixture;
use wireframe::app::WireframeApp;

#[fixture]
#[allow(
    unused_braces,
    reason = "rustc false positive for single line rstest fixtures"
)]
pub fn factory() -> impl Fn() -> WireframeApp + Send + Sync + Clone + 'static {
    || WireframeApp::new().expect("WireframeApp::new failed")
}

#[fixture]
#[allow(
    unused_braces,
    reason = "rustc false positive for single line rstest fixtures"
)]
pub fn unused_port() -> SocketAddr {
    let addr = SocketAddr::new(Ipv4Addr::LOCALHOST.into(), 0);
    let listener = TcpListener::bind(addr).expect("failed to bind port");
    listener.local_addr().expect("failed to obtain local addr")
}
