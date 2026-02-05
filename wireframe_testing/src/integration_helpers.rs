//! Shared helpers for integration testing Wireframe applications.
//!
//! This module exposes a small set of helpers that integration tests can share
//! across multiple test crates. It provides a default envelope type with
//! correlation support, a default app factory, and a helper to bind to an
//! unused local port without relying on file-level lint suppressions.

use std::net::{Ipv4Addr, SocketAddr, TcpListener as StdTcpListener};

use rstest::fixture;
use wireframe::{
    app::{Envelope, Packet, PacketParts},
    correlation::CorrelatableFrame,
    serializer::BincodeSerializer,
};

/// Create a TCP listener bound to a free local port.
///
/// # Errors
///
/// Returns any IO error encountered while binding to an ephemeral localhost
/// port.
///
/// # Examples
///
/// ```rust,no_run
/// use wireframe_testing::{TestResult, unused_listener};
///
/// fn example() -> TestResult {
///     let listener = unused_listener()?;
///     let addr = listener.local_addr()?;
///     assert!(addr.port() > 0);
///     Ok(())
/// }
/// ```
#[must_use]
pub fn unused_listener() -> std::io::Result<StdTcpListener> {
    let addr = SocketAddr::new(Ipv4Addr::LOCALHOST.into(), 0);
    StdTcpListener::bind(addr)
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
    /// Serialised message payload.
    pub payload: Vec<u8>,
}

impl CorrelatableFrame for CommonTestEnvelope {
    fn correlation_id(&self) -> Option<u64> { self.correlation_id }

    fn set_correlation_id(&mut self, correlation_id: Option<u64>) {
        self.correlation_id = correlation_id;
    }
}

impl Packet for CommonTestEnvelope {
    fn id(&self) -> u32 { self.id }

    fn into_parts(self) -> PacketParts {
        PacketParts::new(self.id, self.correlation_id, self.payload)
    }

    fn from_parts(parts: PacketParts) -> Self {
        Self {
            id: parts.id(),
            correlation_id: parts.correlation_id(),
            payload: parts.into_payload(),
        }
    }
}

/// Default app type used by integration test suites.
pub type TestApp = wireframe::app::WireframeApp<BincodeSerializer, (), Envelope>;

/// Shared result type for integration tests.
pub type TestResult<T = ()> = Result<T, Box<dyn std::error::Error + Send + Sync>>;

/// Default `WireframeApp` factory for integration tests.
///
/// # Examples
///
/// ```rust,no_run
/// use wireframe::server::WireframeServer;
/// use wireframe_testing::{TestResult, factory};
///
/// fn example() -> TestResult {
///     let server = WireframeServer::new(factory());
///     assert!(server.worker_count() >= 1);
///     Ok(())
/// }
/// ```
#[fixture]
pub fn factory() -> impl Fn() -> TestApp + Send + Sync + Clone + 'static {
    fn build() -> TestApp { TestApp::default() }
    build
}
