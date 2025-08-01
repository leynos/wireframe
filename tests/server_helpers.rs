//! Utilities shared across server runtime tests.
//!
//! This module provides fixtures and helper constructors used by
//! `server_runtime.rs` and `server_runtime_more.rs`.
use std::net::{Ipv4Addr, SocketAddr};

use bincode::{Decode, Encode};
use rstest::fixture;
use wireframe::{app::WireframeApp, server::WireframeServer};

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
    let listener = std::net::TcpListener::bind(addr).unwrap();
    listener.local_addr().unwrap()
}

/// Create a server bound to the provided address for testing.
///
/// # Panics
///
/// Panics if binding to `addr` fails.
#[allow(dead_code)]
pub fn bind_server<F>(factory: F, addr: SocketAddr) -> WireframeServer<F>
where
    F: Fn() -> WireframeApp + Send + Sync + Clone + 'static,
{
    WireframeServer::new(factory)
        .bind(addr)
        .expect("Failed to bind")
}

#[allow(dead_code)]
pub fn server_with_preamble<F>(factory: F) -> WireframeServer<F, TestPreamble>
where
    F: Fn() -> WireframeApp + Send + Sync + Clone + 'static,
{
    WireframeServer::new(factory).with_preamble::<TestPreamble>()
}
