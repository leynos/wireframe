//! Test helpers shared across server modules.

use std::net::{Ipv4Addr, SocketAddr, TcpListener as StdTcpListener};

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
/// Returns a bound [`TcpListener`] on a free port for use in tests.
///
/// Keeping the listener bound prevents race conditions where another
/// process could claim the port between discovery and use.
pub fn free_listener() -> StdTcpListener {
    let addr = SocketAddr::new(Ipv4Addr::LOCALHOST.into(), 0);
    StdTcpListener::bind(addr).expect("Failed to bind free port listener")
}

/// Extract the bound address from a listener.
///
/// # Examples
///
/// ```
/// use std::net::TcpListener;
///
/// use wireframe::server::test_util::{free_listener, listener_addr};
///
/// let listener = free_listener();
/// let addr = listener_addr(&listener);
/// assert_eq!(listener.local_addr().unwrap(), addr);
/// ```
#[cfg_attr(test, allow(dead_code, reason = "Used via path in tests"))]
#[cfg_attr(not(test), expect(dead_code, reason = "Only used in tests"))]
#[must_use]
pub fn listener_addr(listener: &StdTcpListener) -> SocketAddr {
    listener
        .local_addr()
        .expect("failed to get listener address")
}

pub fn bind_server<F>(factory: F, listener: StdTcpListener) -> WireframeServer<F, (), Bound>
where
    F: Fn() -> WireframeApp + Send + Sync + Clone + 'static,
{
    WireframeServer::new(factory)
        .bind_existing_listener(listener)
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
