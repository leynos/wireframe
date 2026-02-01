//! Active connection counting and RAII guard.

use std::sync::atomic::{AtomicU64, Ordering};

/// Global gauge tracking active connections.
static ACTIVE_CONNECTIONS: AtomicU64 = AtomicU64::new(0);

/// RAII guard incrementing [`ACTIVE_CONNECTIONS`] on creation and
/// decrementing it on drop.
pub(super) struct ActiveConnection;

impl ActiveConnection {
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

/// Return the current number of active connections.
#[must_use]
pub fn active_connection_count() -> u64 { ACTIVE_CONNECTIONS.load(Ordering::Relaxed) }

/// Load the current count for logging purposes.
pub(super) fn current_count() -> u64 { ACTIVE_CONNECTIONS.load(Ordering::Relaxed) }
