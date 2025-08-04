//! Shared utilities for integration tests.
//!
//! Provides fixtures for a basic [`WireframeApp`] factory and a helper to
//! create a TCP listener bound to an unused local port. These helpers reduce
//! duplication across test modules.

use std::net::{Ipv4Addr, SocketAddr, TcpListener as StdTcpListener};

/// Create a TCP listener bound to a free local port.
pub fn unused_listener() -> StdTcpListener {
    let addr = SocketAddr::new(Ipv4Addr::LOCALHOST.into(), 0);
    StdTcpListener::bind(addr).expect("failed to bind port")
}

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
