//! Shared helpers for integration testing Wireframe applications.
//!
//! This module exposes a small set of helpers that integration tests can share
//! across multiple test crates. It provides a default envelope type with
//! correlation support, a default app factory, and a helper to bind to an
//! unused local port without relying on file-level lint suppressions.

use std::net::TcpListener as StdTcpListener;

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
/// fn example() -> TestResult<()> {
///     let listener = unused_listener()?;
///     let addr = listener.local_addr()?;
///     assert!(addr.port() > 0);
///     Ok(())
/// }
/// ```
#[must_use]
pub fn unused_listener() -> std::io::Result<StdTcpListener> { StdTcpListener::bind("localhost:0") }

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

impl CommonTestEnvelope {
    /// Create a new test envelope from raw parts.
    ///
    /// # Examples
    ///
    /// ```
    /// use wireframe_testing::CommonTestEnvelope;
    ///
    /// let envelope = CommonTestEnvelope::new(7, Some(42), vec![1, 2, 3]);
    /// assert_eq!(envelope.id, 7);
    /// ```
    #[must_use]
    pub fn new(id: u32, correlation_id: Option<u64>, payload: Vec<u8>) -> Self {
        Self {
            id,
            correlation_id,
            payload,
        }
    }

    /// Create a new test envelope with no correlation identifier.
    ///
    /// # Examples
    ///
    /// ```
    /// use wireframe_testing::CommonTestEnvelope;
    ///
    /// let envelope = CommonTestEnvelope::with_payload(7, vec![1, 2, 3]);
    /// assert_eq!(envelope.correlation_id, None);
    /// ```
    #[must_use]
    pub fn with_payload(id: u32, payload: Vec<u8>) -> Self {
        Self {
            id,
            correlation_id: None,
            payload,
        }
    }
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
/// fn example() -> TestResult<()> {
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

#[cfg(test)]
mod tests {
    use wireframe::{app::Packet, correlation::CorrelatableFrame};

    use super::{CommonTestEnvelope, factory, unused_listener};

    #[test]
    fn unused_listener_binds_loopback() -> super::TestResult {
        let listener = unused_listener()?;
        let addr = listener.local_addr()?;
        if !addr.ip().is_loopback() {
            return Err(format!("expected loopback address, got {addr:?}").into());
        }
        if addr.port() == 0 {
            return Err("expected a non-zero port".into());
        }
        Ok(())
    }

    #[test]
    fn common_test_envelope_round_trips_parts() -> super::TestResult {
        let mut env = CommonTestEnvelope::new(7, Some(12), vec![1, 2, 3]);
        if env.correlation_id() != Some(12) {
            return Err("expected correlation id to be set".into());
        }
        env.set_correlation_id(Some(99));
        if env.correlation_id() != Some(99) {
            return Err("expected correlation id to update".into());
        }
        let rebuilt = CommonTestEnvelope::from_parts(env.clone().into_parts());
        if rebuilt != env {
            return Err("expected envelope to round trip via parts".into());
        }
        Ok(())
    }

    #[test]
    fn factory_builds_default_app() -> super::TestResult {
        let build = factory();
        let app = build();
        let _codec = app.length_codec();
        Ok(())
    }
}
