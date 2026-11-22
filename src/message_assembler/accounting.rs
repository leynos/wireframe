//! Running accounting and expiry indexing for in-flight assemblies.
//!
//! [`AssemblyTracker`] owns the map of partial assemblies together with the
//! values derived from it: the exact payload bytes retained across every
//! partial and one deadline record per live partial. Keeping the three
//! together makes each mutation update them as one step, so aggregate budgets
//! read a running total instead of rescanning the map on every frame, and
//! expiry visits only the assemblies whose deadlines have actually passed.

use std::{
    collections::{BTreeSet, HashMap},
    time::{Duration, Instant},
};

use super::{MessageKey, budget::MESSAGE_ASSEMBLY_ENTRY_OVERHEAD_BYTES, partial::PartialAssembly};

/// Partial assemblies plus their running byte total and deadline index.
#[derive(Debug, Default)]
pub(super) struct AssemblyTracker {
    /// Live partial assemblies keyed by protocol message key.
    assemblies: HashMap<MessageKey, PartialAssembly>,
    /// Exact payload and metadata bytes retained across every partial.
    buffered_payload_bytes: usize,
    /// One `(started_at, key)` record per live partial, ordered by deadline.
    expiry_index: BTreeSet<(Instant, MessageKey)>,
}

impl AssemblyTracker {
    /// Create an empty tracker.
    pub(super) fn new() -> Self { Self::default() }

    /// Number of live partial assemblies.
    pub(super) fn len(&self) -> usize { self.assemblies.len() }

    /// Whether `key` already has a live partial assembly.
    pub(super) fn contains_key(&self, key: MessageKey) -> bool {
        self.assemblies.contains_key(&key)
    }

    /// Mutably borrow the partial assembly for `key`.
    pub(super) fn get_mut(&mut self, key: MessageKey) -> Option<&mut PartialAssembly> {
        self.assemblies.get_mut(&key)
    }

    /// Insert a newly admitted partial assembly and account for its bytes.
    ///
    /// Callers reject a duplicate key before reaching here, so the map insert
    /// replaces nothing in normal operation. The replacement branch keeps the
    /// accounting exact if that invariant is ever broken.
    pub(super) fn insert(&mut self, key: MessageKey, partial: PartialAssembly) {
        let started_at = partial.started_at();
        let bytes = partial.buffered_bytes();
        if let Some(previous) = self.assemblies.insert(key, partial) {
            debug_assert!(false, "assembly inserted for a key with a live partial");
            self.buffered_payload_bytes -= previous.buffered_bytes();
            self.expiry_index.remove(&(previous.started_at(), key));
        }
        self.buffered_payload_bytes += bytes;
        self.expiry_index.insert((started_at, key));
    }

    /// Append an admitted continuation body and account for its bytes.
    pub(super) fn append_body(&mut self, key: MessageKey, body: &[u8]) {
        let Some(partial) = self.assemblies.get_mut(&key) else {
            debug_assert!(false, "continuation body appended without a live partial");
            return;
        };
        partial.push_body(body);
        self.buffered_payload_bytes += body.len();
    }

    /// Remove the partial assembly for `key`, releasing its bytes and deadline.
    pub(super) fn take(&mut self, key: MessageKey) -> Option<PartialAssembly> {
        let partial = self.assemblies.remove(&key)?;
        self.buffered_payload_bytes -= partial.buffered_bytes();
        self.expiry_index.remove(&(partial.started_at(), key));
        Some(partial)
    }

    /// Remove partials whose age has reached `timeout`, returning their keys.
    ///
    /// The index is ordered by start time, so this visits only the expired
    /// prefix and stops at the first assembly still within its timeout. An
    /// idle purge therefore costs O(1) rather than a scan of every partial.
    pub(super) fn purge_expired_at(&mut self, now: Instant, timeout: Duration) -> Vec<MessageKey> {
        let mut evicted = Vec::new();
        while let Some(&(started_at, key)) = self.expiry_index.first() {
            if now.saturating_duration_since(started_at) < timeout {
                break;
            }
            self.expiry_index.remove(&(started_at, key));
            if let Some(partial) = self.assemblies.remove(&key) {
                self.buffered_payload_bytes -= partial.buffered_bytes();
                evicted.push(key);
            }
        }
        evicted
    }

    /// Exact payload and metadata bytes retained across all partials.
    pub(super) const fn total_buffered_bytes(&self) -> usize { self.buffered_payload_bytes }

    /// Bytes charged against aggregate budgets.
    ///
    /// Retained payload plus a fixed per-entry cost for every live partial, so
    /// an assembly carrying no payload still consumes budget.
    pub(super) fn total_charged_bytes(&self) -> usize {
        self.buffered_payload_bytes.saturating_add(
            self.assemblies
                .len()
                .saturating_mul(MESSAGE_ASSEMBLY_ENTRY_OVERHEAD_BYTES),
        )
    }

    /// Number of deadline records, which must equal the live partial count.
    #[cfg(test)]
    pub(super) fn indexed_deadlines(&self) -> usize { self.expiry_index.len() }

    /// Recompute retained bytes by scanning the map.
    ///
    /// Test-only oracle for the running total.
    #[cfg(test)]
    pub(super) fn recompute_buffered_bytes(&self) -> usize {
        self.assemblies
            .values()
            .map(PartialAssembly::buffered_bytes)
            .sum()
    }
}
