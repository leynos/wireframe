//! Shared helpers for applying transport-level fragmentation to packets.
//!
//! This module re-exports [`fragment_packet`] from the canonical location in
//! [`crate::fragment::packet`] to preserve backward compatibility.

// Re-export from canonical location for backward compatibility.
pub use crate::fragment::fragment_packet;
