//! Shared utilities for integration tests.
//!
//! Provides fixtures for a basic [`WireframeApp`] factory and a helper to
//! create a TCP listener bound to an unused local port. These helpers reduce
//! duplication across test modules.

// File-level suppression required because:
// 1. CommonTestEnvelope is used by some test binaries (routes, lifecycle) but not others
//    (middleware_order), so dead_code fires inconsistently across binaries.
// 2. #[expect(dead_code)] fails in binaries that use the type (unfulfilled expectation).
// 3. #[allow(dead_code)] is rejected by clippy::allow_attributes.
// 4. #[expect(clippy::allow_attributes)] also fires inconsistently across binaries.
// This is a known limitation when sharing test utilities across multiple test binaries.
#![allow(
    clippy::allow_attributes,
    reason = "expect fails inconsistently across test binaries"
)]
#![allow(
    dead_code,
    reason = "shared utilities not used by all importing test binaries"
)]

use std::net::{Ipv4Addr, SocketAddr, TcpListener as StdTcpListener};

use rstest::fixture;
use wireframe::{
    app::{Envelope, Packet, PacketParts},
    correlation::CorrelatableFrame,
    serializer::BincodeSerializer,
};

/// Create a TCP listener bound to a free local port.
#[expect(
    clippy::expect_used,
    reason = "binding to an ephemeral localhost port must abort the test immediately"
)]
pub fn unused_listener() -> StdTcpListener {
    let addr = SocketAddr::new(Ipv4Addr::LOCALHOST.into(), 0);
    StdTcpListener::bind(addr).expect("failed to bind port")
}

/// Shared test envelope type for integration tests.
///
/// Provides a simple envelope implementation with correlation ID support,
/// suitable for testing routing, lifecycle callbacks, and correlation tracking.
#[derive(bincode::Encode, bincode::BorrowDecode, PartialEq, Debug, Clone)]
pub struct CommonTestEnvelope {
    /// Message type identifier for routing.
    pub id: u32,
    /// Optional correlation ID for request/response matching.
    pub correlation_id: Option<u64>,
    /// Serialized message payload.
    pub payload: Vec<u8>,
}

impl CorrelatableFrame for CommonTestEnvelope {
    fn correlation_id(&self) -> Option<u64> { self.correlation_id }

    fn set_correlation_id(&mut self, correlation_id: Option<u64>) {
        self.correlation_id = correlation_id;
    }
}

impl Packet for CommonTestEnvelope {
    #[inline]
    fn id(&self) -> u32 { self.id }

    fn into_parts(self) -> PacketParts {
        PacketParts::new(self.id, self.correlation_id, self.payload)
    }

    fn from_parts(parts: PacketParts) -> Self {
        Self {
            id: parts.id(),
            correlation_id: parts.correlation_id(),
            payload: parts.payload(),
        }
    }
}

/// Default app type used by cucumber worlds during integration tests.
pub type TestApp = wireframe::app::WireframeApp<BincodeSerializer, (), Envelope>;
/// Shared result type for cucumber step implementations.
pub type TestResult<T = ()> = Result<T, Box<dyn std::error::Error + Send + Sync>>;

#[fixture]
pub fn factory() -> impl Fn() -> TestApp + Send + Sync + Clone + 'static {
    fn build() -> TestApp { TestApp::default() }
    build
}

#[cfg(test)]
mod tests {
    #[test]
    fn unused_listener_is_callable() { let _ = super::unused_listener(); }
}
