//! Per-connection memory budget configuration for inbound assembly paths.
//!
//! `MemoryBudgets` stores explicit byte caps configured on
//! [`crate::app::WireframeApp`] via `WireframeApp::memory_budgets(...)`.
//! This module only exposes configuration; enforcement lands in a future
//! iteration.

use std::num::NonZeroUsize;

use serde::{Deserialize, Serialize};

/// Non-zero byte budget wrapper used by memory-budget configuration.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct BudgetBytes(NonZeroUsize);

impl BudgetBytes {
    /// Create a byte budget from a non-zero byte count.
    #[must_use]
    pub const fn new(bytes: NonZeroUsize) -> Self { Self(bytes) }

    /// Return the wrapped non-zero byte count.
    #[must_use]
    pub const fn get(self) -> NonZeroUsize { self.0 }

    /// Return the wrapped byte count as `usize`.
    #[must_use]
    pub const fn as_usize(self) -> usize { self.0.get() }
}

impl From<NonZeroUsize> for BudgetBytes {
    fn from(value: NonZeroUsize) -> Self { Self::new(value) }
}

impl From<BudgetBytes> for NonZeroUsize {
    fn from(value: BudgetBytes) -> Self { value.get() }
}

impl From<BudgetBytes> for usize {
    fn from(value: BudgetBytes) -> Self { value.as_usize() }
}

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
/// use wireframe::app::{BudgetBytes, MemoryBudgets};
///
/// let budgets = MemoryBudgets::new(
///     BudgetBytes::new(NonZeroUsize::new(16 * 1024).expect("non-zero")),
///     BudgetBytes::new(NonZeroUsize::new(64 * 1024).expect("non-zero")),
///     BudgetBytes::new(NonZeroUsize::new(48 * 1024).expect("non-zero")),
/// );
///
/// assert_eq!(budgets.bytes_per_message().as_usize(), 16 * 1024);
/// assert_eq!(budgets.bytes_per_connection().as_usize(), 64 * 1024);
/// assert_eq!(budgets.bytes_in_flight().as_usize(), 48 * 1024);
/// ```
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct MemoryBudgets {
    message_budget: BudgetBytes,
    connection_window: BudgetBytes,
    assembly_bytes: BudgetBytes,
}

impl MemoryBudgets {
    /// Create memory budget limits for one connection.
    #[must_use]
    pub const fn new(
        bytes_per_message: BudgetBytes,
        bytes_per_connection: BudgetBytes,
        bytes_in_flight: BudgetBytes,
    ) -> Self {
        Self {
            message_budget: bytes_per_message,
            connection_window: bytes_per_connection,
            assembly_bytes: bytes_in_flight,
        }
    }

    /// Maximum bytes buffered for a single logical message.
    #[must_use]
    pub const fn bytes_per_message(&self) -> BudgetBytes { self.message_budget }

    /// Maximum bytes buffered for one connection.
    #[must_use]
    pub const fn bytes_per_connection(&self) -> BudgetBytes { self.connection_window }

    /// Maximum bytes buffered across in-flight assemblies on one connection.
    #[must_use]
    pub const fn bytes_in_flight(&self) -> BudgetBytes { self.assembly_bytes }
}
