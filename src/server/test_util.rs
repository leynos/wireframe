//! Test helpers shared across server modules.
use std::{
    io,
    net::{Ipv4Addr, SocketAddr, TcpListener as StdTcpListener},
};

use bincode::{Decode, Encode};
use rstest::fixture;

use super::{Bound, ServerError, WireframeServer};
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
/// Returns a bound [`TcpListener`] on a free port for use in tests.
///
/// Keeping the listener bound prevents race conditions where another
/// process could claim the port between discovery and use.
pub fn free_listener() -> io::Result<StdTcpListener> {
    let addr = SocketAddr::new(Ipv4Addr::LOCALHOST.into(), 0);
    StdTcpListener::bind(addr)
}

/// Reserve a free local port and return its address.
///
/// Creates a temporary listener to obtain an ephemeral port, then immediately
/// drops it so the port may be rebound. This is inherently subject to a
/// time-of-check/time-of-use race; only use in tests.
///
/// # Examples
///
/// ```plaintext
/// let addr = free_addr()?;
/// assert_eq!(addr.ip(), std::net::Ipv4Addr::LOCALHOST.into());
/// ```
#[cfg(test)]
pub fn free_addr() -> io::Result<SocketAddr> {
    let listener = free_listener()?;
    listener_addr(&listener)
}

/// Extract the bound address from a listener.
///
/// # Examples
///
/// ```plaintext
/// use std::net::TcpListener;
///
/// use wireframe::server::test_util::{free_listener, listener_addr};
///
/// let listener = free_listener()?;
/// let addr = listener_addr(&listener)?;
/// assert_eq!(
///     listener.local_addr()?,
///     addr
/// );
/// ```
#[cfg(test)]
pub fn listener_addr(listener: &StdTcpListener) -> io::Result<SocketAddr> { listener.local_addr() }

pub fn bind_server<F>(
    factory: F,
    listener: StdTcpListener,
) -> Result<WireframeServer<F, (), Bound>, ServerError>
where
    F: Fn() -> WireframeApp + Send + Sync + Clone + 'static,
{
    WireframeServer::new(factory).bind_existing_listener(listener)
}

#[cfg(test)]
pub fn server_with_preamble<F>(factory: F) -> WireframeServer<F, TestPreamble>
where
    F: Fn() -> WireframeApp + Send + Sync + Clone + 'static,
{
    WireframeServer::new(factory).with_preamble::<TestPreamble>()
}

#[cfg(test)]
mod tests {
    //! Coverage for server-only listener and preamble fixtures.

    use super::*;

    #[test]
    fn free_addr_uses_localhost() {
        let addr = match free_addr() {
            Ok(addr) => addr,
            Err(error) => panic!("free_addr should succeed: {error}"),
        };
        assert_eq!(addr.ip(), std::net::IpAddr::from(Ipv4Addr::LOCALHOST));
    }

    #[test]
    fn listener_addr_matches_local_addr() {
        let listener = match free_listener() {
            Ok(listener) => listener,
            Err(error) => panic!("free_listener should succeed: {error}"),
        };
        let addr = match listener_addr(&listener) {
            Ok(addr) => addr,
            Err(error) => panic!("listener_addr should succeed: {error}"),
        };
        let local_addr = match listener.local_addr() {
            Ok(addr) => addr,
            Err(error) => panic!("listener.local_addr should succeed: {error}"),
        };
        assert_eq!(addr, local_addr);
    }

    #[test]
    fn server_with_preamble_is_unbound() {
        let server = server_with_preamble(factory());
        assert!(server.local_addr().is_none());
    }
}
