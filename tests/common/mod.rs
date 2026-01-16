//! Shared utilities for integration tests.
//!
//! Provides fixtures for a basic [`WireframeApp`] factory and a helper to
//! create a TCP listener bound to an unused local port. These helpers reduce
//! duplication across test modules.

// Items in this shared module may not be used by all test binaries that import it.
#![allow(
    dead_code,
    reason = "shared test utilities are not used by all test binaries"
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
        let id = parts.id();
        let correlation_id = parts.correlation_id();
        let payload = parts.payload();
        Self {
            id,
            correlation_id,
            payload,
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
