//! Per-connection memory budget configuration for inbound assembly paths.
//!
//! `MemoryBudgets` stores explicit byte caps configured on
//! [`crate::app::WireframeApp`] via `WireframeApp::memory_budgets(...)`.
//! Enforcement is applied by downstream inbound assembly components.

use std::num::NonZeroUsize;

/// Explicit byte caps for per-connection inbound buffering.
///
/// The three limits map to ADR 0002 budget dimensions:
///
/// - bytes buffered per message;
/// - bytes buffered per connection; and
/// - bytes buffered across in-flight assemblies.
///
/// # Examples
///
/// ```
/// use std::num::NonZeroUsize;
///
/// use wireframe::app::MemoryBudgets;
///
/// let budgets = MemoryBudgets::new(
///     NonZeroUsize::new(16 * 1024).expect("non-zero"),
///     NonZeroUsize::new(64 * 1024).expect("non-zero"),
///     NonZeroUsize::new(48 * 1024).expect("non-zero"),
/// );
///
/// assert_eq!(budgets.bytes_per_message().get(), 16 * 1024);
/// assert_eq!(budgets.bytes_per_connection().get(), 64 * 1024);
/// assert_eq!(budgets.bytes_in_flight().get(), 48 * 1024);
/// ```
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct MemoryBudgets {
    message_budget: NonZeroUsize,
    connection_window: NonZeroUsize,
    assembly_bytes: NonZeroUsize,
}

impl MemoryBudgets {
    /// Create memory budget limits for one connection.
    #[must_use]
    pub const fn new(
        bytes_per_message: NonZeroUsize,
        bytes_per_connection: NonZeroUsize,
        bytes_in_flight: NonZeroUsize,
    ) -> Self {
        Self {
            message_budget: bytes_per_message,
            connection_window: bytes_per_connection,
            assembly_bytes: bytes_in_flight,
        }
    }

    /// Maximum bytes buffered for a single logical message.
    #[must_use]
    pub const fn bytes_per_message(&self) -> NonZeroUsize { self.message_budget }

    /// Maximum bytes buffered for one connection.
    #[must_use]
    pub const fn bytes_per_connection(&self) -> NonZeroUsize { self.connection_window }

    /// Maximum bytes buffered across in-flight assemblies on one connection.
    #[must_use]
    pub const fn bytes_in_flight(&self) -> NonZeroUsize { self.assembly_bytes }
}
