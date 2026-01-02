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
