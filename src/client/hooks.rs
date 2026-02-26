//! Client connection lifecycle and request hooks.
//!
//! This module defines callback types for client-side connection lifecycle
//! events and per-request middleware hooks. Lifecycle hooks fire at connection
//! boundaries (setup, teardown, error). Request hooks fire on every outgoing
//! frame and every incoming frame, enabling symmetric instrumentation with
//! the server's middleware stack.

use std::{future::Future, pin::Pin, sync::Arc};

use bytes::BytesMut;

use super::ClientError;

/// A boxed future that is `Send` with a specified lifetime.
///
/// This type alias reduces verbosity in handler type signatures.
pub type BoxFuture<'a, T> = Pin<Box<dyn Future<Output = T> + Send + 'a>>;

/// Handler invoked when a client connection is established.
///
/// Called after the TCP connection is established and preamble exchange (if
/// configured) succeeds. The handler can perform authentication or other setup
/// tasks and returns connection-specific state stored for the connection's
/// lifetime.
///
/// # Examples
///
/// ```rust,no_run
/// use std::sync::Arc;
///
/// use wireframe::client::ClientConnectionSetupHandler;
///
/// struct SessionState {
///     session_id: u64,
/// }
///
/// let setup: ClientConnectionSetupHandler<SessionState> =
///     Arc::new(|| Box::pin(async { SessionState { session_id: 42 } }));
/// ```
pub type ClientConnectionSetupHandler<C> = Arc<dyn Fn() -> BoxFuture<'static, C> + Send + Sync>;

/// Handler invoked when a client connection is closed.
///
/// Called when the connection is explicitly closed via
/// [`WireframeClient::close`](super::WireframeClient::close). The handler
/// receives the connection state produced by the setup handler.
///
/// # Examples
///
/// ```rust,no_run
/// use std::sync::Arc;
///
/// use wireframe::client::ClientConnectionTeardownHandler;
///
/// struct SessionState {
///     session_id: u64,
/// }
///
/// let teardown: ClientConnectionTeardownHandler<SessionState> = Arc::new(|state| {
///     Box::pin(async move {
///         println!("Session {} closed", state.session_id);
///     })
/// });
/// ```
pub type ClientConnectionTeardownHandler<C> =
    Arc<dyn Fn(C) -> BoxFuture<'static, ()> + Send + Sync>;

/// Handler invoked when the client encounters an error.
///
/// Called when an error occurs during send, receive, or call operations. The
/// handler receives a reference to the error for logging or recovery actions.
/// The handler is invoked before the error is returned to the caller.
///
/// # Examples
///
/// ```rust,no_run
/// use std::sync::Arc;
///
/// use wireframe::client::ClientErrorHandler;
///
/// let error_handler: ClientErrorHandler = Arc::new(|err| {
///     Box::pin(async move {
///         eprintln!("Client error: {err}");
///     })
/// });
/// ```
pub type ClientErrorHandler =
    Arc<dyn for<'a> Fn(&'a ClientError) -> BoxFuture<'a, ()> + Send + Sync>;

/// Configuration for client lifecycle hooks.
///
/// This struct holds the optional lifecycle callbacks configured via the
/// builder. It is used internally to pass hook configuration from the builder
/// to the client runtime.
#[expect(
    clippy::struct_field_names,
    reason = "on_ prefix is idiomatic for callback fields"
)]
pub(crate) struct LifecycleHooks<C> {
    /// Callback invoked when the connection is established.
    pub(crate) on_connect: Option<ClientConnectionSetupHandler<C>>,
    /// Callback invoked when the connection is closed.
    pub(crate) on_disconnect: Option<ClientConnectionTeardownHandler<C>>,
    /// Callback invoked when an error occurs.
    pub(crate) on_error: Option<ClientErrorHandler>,
}

impl<C> Default for LifecycleHooks<C> {
    fn default() -> Self {
        Self {
            on_connect: None,
            on_disconnect: None,
            on_error: None,
        }
    }
}

/// Hook invoked after serialisation, before a frame is written to the
/// transport.
///
/// The hook receives a mutable reference to the serialised bytes, allowing
/// inspection or modification (e.g., prepending an authentication token,
/// logging frame size, incrementing a metrics counter).
///
/// # Examples
///
/// ```rust
/// use std::sync::{
///     Arc,
///     atomic::{AtomicUsize, Ordering},
/// };
///
/// use wireframe::client::BeforeSendHook;
///
/// let counter = Arc::new(AtomicUsize::new(0));
/// let count = counter.clone();
/// let hook: BeforeSendHook = Arc::new(move |_bytes: &mut Vec<u8>| {
///     count.fetch_add(1, Ordering::Relaxed);
/// });
/// ```
pub type BeforeSendHook = Arc<dyn Fn(&mut Vec<u8>) + Send + Sync>;

/// Hook invoked after a frame is read from the transport, before
/// deserialisation.
///
/// The hook receives a mutable reference to the raw bytes, allowing
/// inspection or modification before the deserialiser processes them.
///
/// # Examples
///
/// ```rust
/// use std::sync::{
///     Arc,
///     atomic::{AtomicUsize, Ordering},
/// };
///
/// use wireframe::client::AfterReceiveHook;
///
/// let counter = Arc::new(AtomicUsize::new(0));
/// let count = counter.clone();
/// let hook: AfterReceiveHook = Arc::new(move |_bytes: &mut bytes::BytesMut| {
///     count.fetch_add(1, Ordering::Relaxed);
/// });
/// ```
pub type AfterReceiveHook = Arc<dyn Fn(&mut BytesMut) + Send + Sync>;

/// Configuration for client request/response hooks.
///
/// Stores hooks that fire on every outgoing frame (after serialisation) and
/// every incoming frame (before deserialisation). Multiple hooks of each type
/// may be registered; they execute in registration order.
#[derive(Default)]
pub(crate) struct RequestHooks {
    /// Hooks invoked before each outgoing frame is sent.
    pub(crate) before_send: Vec<BeforeSendHook>,
    /// Hooks invoked after each incoming frame is received.
    pub(crate) after_receive: Vec<AfterReceiveHook>,
}
