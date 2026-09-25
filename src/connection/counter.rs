//! Active connection counting and RAII guard.

#[cfg(not(loom))]
use std::sync::atomic::{AtomicU64, Ordering};

#[cfg(loom)]
use loom::sync::atomic::{AtomicU64, Ordering};

/// Global gauge tracking active connections.
#[cfg(not(loom))]
static ACTIVE_CONNECTIONS: AtomicU64 = AtomicU64::new(0);

// Loom's atomics have no `const` constructor, and Loom must see the gauge to
// schedule the guards that move it, so under `cfg(loom)` the gauge is a
// Loom-managed static, created afresh for every explored execution.
#[cfg(loom)]
loom::lazy_static! {
    /// Global gauge tracking active connections, as Loom sees it.
    static ref ACTIVE_CONNECTIONS: AtomicU64 = AtomicU64::new(0);
}

/// RAII guard incrementing [`ACTIVE_CONNECTIONS`] on creation and
/// decrementing it on drop.
pub(super) struct ActiveConnection;

impl ActiveConnection {
    /// Register one live actor before it begins polling connection sources.
    pub(super) fn new() -> Self {
        ACTIVE_CONNECTIONS.fetch_add(1, Ordering::Relaxed);
        crate::metrics::inc_connections();
        Self
    }
}

impl Drop for ActiveConnection {
    fn drop(&mut self) {
        ACTIVE_CONNECTIONS.fetch_sub(1, Ordering::Relaxed);
        crate::metrics::dec_connections();
    }
}

/// The connection actor's gauge guard, exposed so Loom models can hold one
/// without building an actor.
///
/// Only present under `cfg(loom)`; the production API is unchanged.
///
/// # Examples
///
/// ```ignore
/// let guard = wireframe::connection::LoomConnectionGuard::new();
/// assert_eq!(wireframe::connection::active_connection_count(), 1);
/// drop(guard);
/// ```
#[cfg(loom)]
pub struct LoomConnectionGuard(ActiveConnection);

#[cfg(loom)]
impl LoomConnectionGuard {
    /// Register one live connection, exactly as the actor does.
    #[must_use]
    pub fn new() -> Self { Self(ActiveConnection::new()) }
}

/// Return the current number of active connections.
#[must_use]
pub fn active_connection_count() -> u64 { ACTIVE_CONNECTIONS.load(Ordering::Relaxed) }

/// Load the current count for logging purposes.
pub(super) fn current_count() -> u64 { ACTIVE_CONNECTIONS.load(Ordering::Relaxed) }
