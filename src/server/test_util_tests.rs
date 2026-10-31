//! Coverage for server-only listener and preamble fixtures.

use super::*;

#[test]
fn free_addr_uses_localhost() {
    let addr = free_addr().expect("free_addr should succeed");
    assert_eq!(addr.ip(), std::net::IpAddr::from(Ipv4Addr::LOCALHOST));
}

#[test]
fn listener_addr_matches_local_addr() {
    let listener = free_listener().expect("free_listener should succeed");
    let addr = listener_addr(&listener).expect("listener_addr should succeed");
    let local_addr = listener
        .local_addr()
        .expect("listener.local_addr should succeed");
    assert_eq!(addr, local_addr);
}

#[test]
fn server_with_preamble_is_unbound() {
    let server = server_with_preamble(factory());
    assert!(server.local_addr().is_none());
}
