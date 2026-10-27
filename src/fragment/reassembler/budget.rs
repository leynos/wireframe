//! Shared admission and accounting helpers for reassembly buffers.

use std::{collections::hash_map::OccupiedEntry, num::NonZeroUsize};

use super::PartialMessage;
use crate::fragment::{MessageId, ReassemblyError};

/// Per-connection limits and exact accounting for retained partial payloads.
#[derive(Debug)]
pub(super) struct ReassemblyBudget {
    /// Hard cap preventing one reconstructed message from growing unbounded.
    max_message_size: NonZeroUsize,
    /// Optional cap preventing too many incomplete messages at once.
    max_concurrent_partials: Option<NonZeroUsize>,
    /// Optional cap on payload bytes retained across all incomplete messages.
    max_buffered_bytes: Option<NonZeroUsize>,
    /// Exact total payload bytes currently retained in the reassembly map.
    buffered_bytes: usize,
}

impl ReassemblyBudget {
    /// Create a fresh budget with no retained payload bytes.
    pub(super) const fn new(
        max_message_size: NonZeroUsize,
        max_concurrent_partials: Option<NonZeroUsize>,
        max_buffered_bytes: Option<NonZeroUsize>,
    ) -> Self {
        Self {
            max_message_size,
            max_concurrent_partials,
            max_buffered_bytes,
            buffered_bytes: 0,
        }
    }

    /// Reject a message that exceeds the configured per-message cap.
    pub(super) const fn assert_within_message_limit(
        &self,
        message_id: MessageId,
        attempted: usize,
    ) -> Result<(), ReassemblyError> {
        if attempted > self.max_message_size.get() {
            return Err(ReassemblyError::MessageTooLarge {
                message_id,
                attempted,
                limit: self.max_message_size,
            });
        }
        Ok(())
    }

    /// Return the per-message size cap for overflow reporting.
    pub(super) const fn max_message_size(&self) -> NonZeroUsize { self.max_message_size }

    /// Reject a newly retained message when the concurrent partial cap is full.
    pub(super) fn assert_within_partial_limit(
        &self,
        buffered_partials: usize,
    ) -> Result<(), ReassemblyError> {
        if let Some(limit) = self.max_concurrent_partials {
            let attempted = buffered_partials.saturating_add(1);
            if attempted > limit.get() {
                return Err(ReassemblyError::TooManyConcurrentPartials { attempted, limit });
            }
        }
        Ok(())
    }

    /// Add retained bytes when the aggregate cap permits the growth.
    pub(super) fn add_bytes(&mut self, additional: usize) -> Result<(), ReassemblyError> {
        let Some(attempted) = self.buffered_bytes.checked_add(additional) else {
            return Err(ReassemblyError::AggregateBufferCapExceeded {
                attempted: usize::MAX,
                limit: self.max_buffered_bytes.unwrap_or(NonZeroUsize::MAX),
            });
        };
        if let Some(limit) = self.max_buffered_bytes
            && attempted > limit.get()
        {
            return Err(ReassemblyError::AggregateBufferCapExceeded { attempted, limit });
        }
        self.buffered_bytes = attempted;
        Ok(())
    }

    /// Release bytes when a partial leaves the in-flight map.
    pub(super) fn release_bytes(&mut self, bytes: usize) {
        debug_assert!(
            self.buffered_bytes >= bytes,
            "reassembly byte count underflow"
        );
        self.buffered_bytes -= bytes;
    }

    /// Remove a partial and release its contribution from aggregate accounting.
    pub(super) fn remove_partial(
        &mut self,
        occupied: OccupiedEntry<'_, MessageId, PartialMessage>,
    ) -> PartialMessage {
        let partial = occupied.remove();
        self.release_bytes(partial.len());
        partial
    }

    /// Return the number of retained payload bytes.
    pub(super) const fn buffered_bytes(&self) -> usize { self.buffered_bytes }
}
