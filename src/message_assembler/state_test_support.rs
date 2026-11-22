//! Test-only oracles for the running totals and expiry index.
//!
//! These live in a child module so [`MessageAssemblyState`] keeps its public
//! surface and stays inside the repository's file-size budget. They compare the
//! incrementally maintained values against full recomputations.

use super::MessageAssemblyState;

impl MessageAssemblyState {
    /// Recompute retained bytes by scanning the assembly map.
    ///
    /// The running total must always match this value.
    pub(crate) fn recompute_buffered_bytes(&self) -> usize {
        self.assemblies.recompute_buffered_bytes()
    }

    /// Number of deadline records held by the expiry index.
    ///
    /// This must always match [`Self::buffered_count`].
    pub(crate) fn indexed_deadlines(&self) -> usize { self.assemblies.indexed_deadlines() }
}
