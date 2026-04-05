//! Shared helpers for integration testing Wireframe applications.
//!
//! This module exposes a small set of helpers that integration tests can share
//! across multiple test crates. It provides a default envelope type with
//! correlation support, a default app factory, and a helper to bind to an
//! unused local port without relying on file-level lint suppressions.

use std::{
    net::TcpListener as StdTcpListener,
    pin::Pin,
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    },
};

use rstest::fixture;
pub use wireframe::testkit::{TestError, TestResult};
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

/// Minimal payload type for echo-style round-trip tests.
///
/// Carries a single `u8` and derives the bincode traits needed for
/// serialization through [`wireframe::message::Message`].
///
/// [`wireframe::message::Message`]: wireframe::message::Message
///
/// # Examples
///
/// ```
/// use wireframe::message::Message;
/// use wireframe_testing::Echo;
///
/// let bytes = Echo(42).to_bytes().unwrap();
/// let (decoded, _) = Echo::from_bytes(&bytes).unwrap();
/// assert_eq!(decoded, Echo(42));
/// ```
#[derive(bincode::Encode, bincode::BorrowDecode, PartialEq, Debug)]
pub struct Echo(pub u8);

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

/// App type parameterized over [`CommonTestEnvelope`] for pair-harness and
/// integration tests that need public field access.
pub type CommonTestApp = wireframe::app::WireframeApp<BincodeSerializer, (), CommonTestEnvelope>;

/// Handler type alias for [`CommonTestEnvelope`] handlers.
pub type CommonHandler = Arc<
    dyn Fn(&CommonTestEnvelope) -> Pin<Box<dyn std::future::Future<Output = ()> + Send>>
        + Send
        + Sync,
>;

/// Build a handler that counts invocations without modifying the response.
///
/// The [`WireframeApp`](wireframe::app::WireframeApp) echoes the envelope
/// back automatically, so this handler only records that the route was
/// invoked. Pass an [`AtomicUsize`] counter to observe invocation counts
/// in assertions.
///
/// # Examples
///
/// ```rust,no_run
/// use std::sync::{Arc, atomic::AtomicUsize};
///
/// use wireframe_testing::echo_handler;
///
/// let counter = Arc::new(AtomicUsize::new(0));
/// let handler = echo_handler(&counter);
/// ```
pub fn echo_handler(counter: &Arc<AtomicUsize>) -> CommonHandler {
    let counter = counter.clone();
    Arc::new(move |_: &CommonTestEnvelope| {
        let c = counter.clone();
        Box::pin(async move {
            c.fetch_add(1, Ordering::SeqCst);
        })
    })
}

/// Build a factory closure that produces a single-route echo app.
///
/// The returned closure can be passed directly to
/// [`spawn_wireframe_pair`](crate::spawn_wireframe_pair) or
/// [`spawn_wireframe_pair_default`](crate::spawn_wireframe_pair_default).
/// Each call produces a [`CommonTestApp`] with one route (message id `1`)
/// that counts invocations via the shared `counter`.
///
/// The closure returns a [`TestResult`] so build failures surface as
/// errors rather than panics.
///
/// # Examples
///
/// ```rust,no_run
/// use std::sync::{Arc, atomic::AtomicUsize};
///
/// use wireframe_testing::echo_app_factory;
///
/// let counter = Arc::new(AtomicUsize::new(0));
/// let factory = echo_app_factory(&counter);
/// ```
pub fn echo_app_factory(
    counter: &Arc<AtomicUsize>,
) -> impl Fn() -> TestResult<CommonTestApp> + Send + Sync + Clone + 'static {
    let handler = echo_handler(counter);
    move || {
        let app = CommonTestApp::new()?;
        Ok(app.route(1, handler.clone())?)
    }
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
