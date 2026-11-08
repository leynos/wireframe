//! Connection lifecycle hooks for `WireframeApp`.

use std::{future::Future, pin::Pin};

/// Callback invoked when a connection is established.
///
/// # Examples
///
/// ```rust,no_run
/// use std::sync::Arc;
///
/// use wireframe::app::ConnectionSetup;
/// let setup: Arc<ConnectionSetup<String>> =
///     Arc::new(|| Box::pin(async { String::from("hello") }));
/// ```
pub type ConnectionSetup<C> = dyn Fn() -> Pin<Box<dyn Future<Output = C> + Send>> + Send + Sync;

/// Callback invoked when a connection is closed.
///
/// When an application registers this callback and a matching
/// [`ConnectionSetup`] has produced state, teardown runs exactly once after
/// the connection stops processing. That guarantee covers clean completion,
/// stream-processing errors such as malformed frames or failed sends, a
/// handler panic caught by the connection runtime, and cooperative
/// cancellation observed at a read boundary.
///
/// Teardown does not run when setup was absent, failed, or never completed, nor
/// when the connection future is dropped or aborted before processing stops.
/// A panic during setup or teardown is not retried. Builds using
/// `panic = "abort"` cannot recover a handler panic for finalization. See
/// [ADR-012](../../docs/adr-012-prepared-application-and-connection-runtime.md)
/// for the full contract and its accepted gaps.
///
/// # Examples
///
/// ```rust,no_run
/// use std::sync::Arc;
///
/// use wireframe::app::ConnectionTeardown;
/// let teardown: Arc<ConnectionTeardown<String>> = Arc::new(|state| {
///     Box::pin(async move {
///         println!("Dropping {state}");
///     })
/// });
/// ```
pub type ConnectionTeardown<C> =
    dyn Fn(C) -> Pin<Box<dyn Future<Output = ()> + Send>> + Send + Sync;
