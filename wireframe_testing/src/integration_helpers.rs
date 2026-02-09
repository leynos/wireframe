//! Shared helpers for integration testing Wireframe applications.
//!
//! This module exposes a small set of helpers that integration tests can share
//! across multiple test crates. It provides a default envelope type with
//! correlation support, a default app factory, and a helper to bind to an
//! unused local port without relying on file-level lint suppressions.

use std::net::TcpListener as StdTcpListener;

use rstest::fixture;
use thiserror::Error;
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
pub fn unused_listener() -> TestResult<StdTcpListener> { Ok(StdTcpListener::bind("localhost:0")?) }

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

    /// Create a new test envelope with an identifier and no payload.
    ///
    /// # Examples
    ///
    /// ```
    /// use wireframe_testing::CommonTestEnvelope;
    ///
    /// let envelope = CommonTestEnvelope::with_id(7);
    /// assert_eq!(envelope.payload, Vec::<u8>::new());
    /// ```
    #[must_use]
    pub fn with_id(id: u32) -> Self {
        Self {
            id,
            correlation_id: None,
            payload: Vec::new(),
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

/// Error type for integration test helpers.
#[derive(Debug, Error)]
pub enum TestError {
    /// IO error surfaced while exercising test utilities.
    #[error("io error: {0}")]
    Io(#[from] std::io::Error),
    /// Assertion or other message-driven failure.
    #[error("{0}")]
    Msg(String),
    #[error(transparent)]
    Wireframe(#[from] wireframe::app::WireframeError),
    #[error(transparent)]
    Client(#[from] wireframe::client::ClientError),
    #[error(transparent)]
    Server(#[from] wireframe::server::ServerError),
    #[error(transparent)]
    Push(#[from] wireframe::push::PushError),
    #[error(transparent)]
    PushConfig(#[from] wireframe::push::PushConfigError),
    #[error(transparent)]
    ConnectionState(#[from] wireframe::connection::ConnectionStateError),
    #[error(transparent)]
    Reassembly(#[from] wireframe::ReassemblyError),
    #[error(transparent)]
    Fragmentation(#[from] wireframe::FragmentationError),
    #[error(transparent)]
    Codec(#[from] wireframe::codec::CodecError),
    #[error(transparent)]
    Encode(#[from] bincode::error::EncodeError),
    #[error(transparent)]
    Decode(#[from] bincode::error::DecodeError),
    #[error(transparent)]
    Join(#[from] tokio::task::JoinError),
    #[error(transparent)]
    Timeout(#[from] tokio::time::error::Elapsed),
    #[error(transparent)]
    OneshotRecv(#[from] tokio::sync::oneshot::error::RecvError),
    #[error(transparent)]
    OneshotTryRecv(#[from] tokio::sync::oneshot::error::TryRecvError),
    #[error(transparent)]
    MpscTryRecv(#[from] tokio::sync::mpsc::error::TryRecvError),
    #[error(transparent)]
    ParseInt(#[from] std::num::ParseIntError),
    #[error(transparent)]
    TryFromInt(#[from] std::num::TryFromIntError),
    #[error(transparent)]
    Utf8(#[from] std::str::Utf8Error),
    #[error(transparent)]
    FromUtf8(#[from] std::string::FromUtf8Error),
    #[error(transparent)]
    AddrParse(#[from] std::net::AddrParseError),
}

impl From<String> for TestError {
    fn from(value: String) -> Self { Self::Msg(value) }
}

impl From<&str> for TestError {
    fn from(value: &str) -> Self { Self::Msg(value.to_string()) }
}

impl<T> From<tokio::sync::mpsc::error::SendError<T>> for TestError {
    fn from(err: tokio::sync::mpsc::error::SendError<T>) -> Self { Self::Msg(err.to_string()) }
}

impl<T> From<tokio::sync::mpsc::error::TrySendError<T>> for TestError {
    fn from(err: tokio::sync::mpsc::error::TrySendError<T>) -> Self { Self::Msg(err.to_string()) }
}

impl From<Box<dyn std::error::Error + Send + Sync>> for TestError {
    fn from(err: Box<dyn std::error::Error + Send + Sync>) -> Self { Self::Msg(err.to_string()) }
}

/// Shared result type for integration tests.
pub type TestResult<T = ()> = Result<T, TestError>;

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
        let empty = CommonTestEnvelope::with_id(5);
        if empty.id != 5 {
            return Err("expected id to be set".into());
        }
        if empty.correlation_id.is_some() {
            return Err("expected no correlation id".into());
        }
        if !empty.payload.is_empty() {
            return Err("expected empty payload".into());
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
