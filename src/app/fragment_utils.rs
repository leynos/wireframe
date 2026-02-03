//! Shared helpers for applying transport-level fragmentation to packets.
//!
//! **Deprecated:** This module re-exports [`fragment_packet`] from its
//! canonical location in [`crate::fragment`]. New code should import directly
//! from [`crate::fragment::fragment_packet`] instead.

// Re-export from canonical location for backward compatibility.
#[deprecated(
    since = "0.3.0",
    note = "use `crate::fragment::fragment_packet` instead"
)]
pub use crate::fragment::fragment_packet;
