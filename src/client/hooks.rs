//! Client connection lifecycle hooks.
//!
//! This module defines callback types for client-side connection lifecycle
//! events that mirror the server's hook system. These hooks enable consistent
//! instrumentation and middleware integration across both client and server.

use std::{future::Future, pin::Pin, sync::Arc};

use super::ClientError;

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
pub type ClientConnectionSetupHandler<C> =
    Arc<dyn Fn() -> Pin<Box<dyn Future<Output = C> + Send>> + Send + Sync>;

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
    Arc<dyn Fn(C) -> Pin<Box<dyn Future<Output = ()> + Send>> + Send + Sync>;

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
pub type ClientErrorHandler = Arc<
    dyn for<'a> Fn(&'a ClientError) -> Pin<Box<dyn Future<Output = ()> + Send + 'a>> + Send + Sync,
>;

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
