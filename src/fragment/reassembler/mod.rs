//! Inbound helper that stitches fragments back into complete messages.
//!
//! [`Reassembler`] mirrors the outbound [`Fragmenter`](crate::fragment::Fragmenter) by
//! collecting fragment payloads keyed by [`MessageId`].
//! It enforces ordering via [`FragmentSeries`], guards
//! against unbounded allocation with a configurable cap, and purges stale partial
//! assemblies after a fixed timeout. The helper is transport-agnostic so codecs and
//! behavioural tests can reuse it without depending on socket types.

use std::{
    collections::{
        HashMap,
        hash_map::{Entry, OccupiedEntry},
    },
    num::NonZeroUsize,
    time::{Duration, Instant},
};

use bincode::error::DecodeError;

use super::{FragmentHeader, FragmentSeries, FragmentStatus, MessageId, ReassemblyError};
use crate::message::Message;

mod budget;
mod first_fragment;

/// Owned state retained while a message's remaining fragments arrive.
#[derive(Debug)]
struct PartialMessage {
    /// Ordering tracker for fragments already accepted into this buffer.
    series: FragmentSeries,
    /// Concatenated payload, retained until the final fragment arrives.
    buffer: Vec<u8>,
    /// Clock reading used to evict abandoned assemblies.
    started_at: Instant,
}

/// Borrowed inputs used when registering the first fragment of a series.
struct FirstFragment<'a> {
    /// Header establishing the message identity and initial ordering state.
    header: FragmentHeader,
    /// Payload borrowed only for the duration of insertion and validation.
    payload: &'a [u8],
    /// Timestamp that anchors timeout-based eviction for this series.
    now: Instant,
}

impl PartialMessage {
    /// Start a partial message with its first payload and timeout timestamp.
    fn new(series: FragmentSeries, payload: &[u8], started_at: Instant) -> Self {
        Self {
            series,
            buffer: payload.to_vec(),
            started_at,
        }
    }

    /// Append an ordered fragment payload to the owned reassembly buffer.
    fn push(&mut self, payload: &[u8]) { self.buffer.extend_from_slice(payload); }

    /// Return the number of payload bytes currently buffered.
    const fn len(&self) -> usize { self.buffer.len() }

    /// Return the timestamp at which this partial series began.
    const fn started_at(&self) -> Instant { self.started_at }

    /// Release the completed payload buffer to avoid a second allocation.
    fn into_buffer(self) -> Vec<u8> { self.buffer }
}

/// Container for a fully re-assembled message payload.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ReassembledMessage {
    /// Identity of the logical message reconstructed from the fragments.
    message_id: MessageId,
    /// Contiguous payload bytes in protocol order.
    payload: Vec<u8>,
}

impl ReassembledMessage {
    /// Construct a new [`ReassembledMessage`].
    #[must_use]
    pub const fn new(message_id: MessageId, payload: Vec<u8>) -> Self {
        Self {
            message_id,
            payload,
        }
    }

    /// Identifier shared by the fragments that formed this message.
    #[must_use]
    pub const fn message_id(&self) -> MessageId { self.message_id }

    /// Borrow the re-assembled payload.
    #[must_use]
    pub const fn payload(&self) -> &[u8] { self.payload.as_slice() }

    /// Consume the message, returning the owned payload bytes.
    #[must_use]
    pub fn into_payload(self) -> Vec<u8> { self.payload }

    /// Decode the payload into a strongly typed message.
    ///
    /// # Errors
    ///
    /// Returns any [`DecodeError`] raised while deserializing the payload.
    pub fn decode<M: Message>(&self) -> Result<M, DecodeError> {
        let (message, _) = M::from_bytes(self.payload())?;
        Ok(message)
    }
}

/// Stateful fragment re-assembler with timeout-based eviction.
#[derive(Debug)]
pub struct Reassembler {
    /// Per-message and aggregate admission limits with retained-byte accounting.
    budget: budget::ReassemblyBudget,
    /// Age after which an incomplete series is discarded.
    timeout: Duration,
    /// In-flight buffers keyed by logical message identity.
    buffers: HashMap<MessageId, PartialMessage>,
}

impl Reassembler {
    /// Create a re-assembler with per-message and optional aggregate limits.
    ///
    /// Set either aggregate limit to `None` to disable that dimension.
    ///
    /// # Examples
    ///
    /// ```
    /// use std::{num::NonZeroUsize, time::Duration};
    ///
    /// use wireframe::fragment::{FragmentHeader, FragmentIndex, MessageId, Reassembler};
    /// let cap = NonZeroUsize::new(1024).expect("non-zero cap");
    /// let mut reassembler = Reassembler::new(cap, Duration::from_secs(30), Some(cap), Some(cap));
    /// let first = FragmentHeader::new(MessageId::new(7), FragmentIndex::zero(), false);
    /// reassembler
    ///     .push(first, [1_u8, 2, 3])
    ///     .expect("first fragment should be accepted");
    /// assert_eq!(reassembler.buffered_bytes(), 3);
    /// ```
    #[must_use]
    pub fn new(
        max_message_size: NonZeroUsize,
        timeout: Duration,
        max_concurrent_partials: Option<NonZeroUsize>,
        max_buffered_bytes: Option<NonZeroUsize>,
    ) -> Self {
        Self {
            budget: budget::ReassemblyBudget::new(
                max_message_size,
                max_concurrent_partials,
                max_buffered_bytes,
            ),
            timeout,
            buffers: HashMap::new(),
        }
    }

    /// Process a fragment using the current time.
    ///
    /// Returns `Ok(Some(_))` when the fragment completes the message, `Ok(None)`
    /// while more fragments are required, or an error if ordering or size
    /// invariants are violated.
    ///
    /// # Errors
    ///
    /// Returns [`ReassemblyError`] when a fragment arrives out of order or would
    /// push the reconstructed payload beyond the configured limit.
    pub fn push(
        &mut self,
        header: FragmentHeader,
        payload: impl AsRef<[u8]>,
    ) -> Result<Option<ReassembledMessage>, ReassemblyError> {
        self.push_at(header, payload, Instant::now())
    }

    /// Process a fragment using an explicit clock reading.
    ///
    /// Accepting an explicit `now` simplifies deterministic testing and allows
    /// callers to co-ordinate eviction sweeps with their own timers.
    ///
    /// # Errors
    ///
    /// Returns [`ReassemblyError`] when the fragment violates ordering or
    /// exceeds the configured reassembly cap.
    pub fn push_at(
        &mut self,
        header: FragmentHeader,
        payload: impl AsRef<[u8]>,
        now: Instant,
    ) -> Result<Option<ReassembledMessage>, ReassemblyError> {
        self.purge_expired_at(now);

        let payload_bytes = payload.as_ref();
        let buffered_partials = self.buffers.len();
        let budget = &mut self.budget;

        match self.buffers.entry(header.message_id()) {
            Entry::Occupied(occupied) => {
                Self::push_existing_fragment(budget, occupied, header, payload_bytes)
            }
            Entry::Vacant(vacant) => first_fragment::push_first_fragment(
                budget,
                buffered_partials,
                vacant,
                &FirstFragment {
                    header,
                    payload: payload_bytes,
                    now,
                },
            ),
        }
    }

    /// Remove any partial messages that exceeded the configured timeout.
    ///
    /// Returns the identifiers of messages that were evicted.
    pub fn purge_expired(&mut self) -> Vec<MessageId> { self.purge_expired_at(Instant::now()) }

    /// Remove any partial messages that exceeded the configured timeout using
    /// an explicit clock reading.
    ///
    /// Returns the identifiers of messages that were evicted.
    pub fn purge_expired_at(&mut self, now: Instant) -> Vec<MessageId> {
        let mut evicted = Vec::new();
        let timeout = self.timeout;
        let budget = &mut self.budget;

        self.buffers.retain(|message_id, partial| {
            let expired = now.saturating_duration_since(partial.started_at()) >= timeout;
            if expired {
                evicted.push(*message_id);
                budget.release_bytes(partial.len());
            }
            !expired
        });

        evicted
    }

    /// Number of partial messages currently buffered.
    #[must_use]
    pub fn buffered_len(&self) -> usize { self.buffers.len() }

    /// Number of payload bytes currently retained across all partial messages.
    ///
    /// # Examples
    ///
    /// ```
    /// use std::{num::NonZeroUsize, time::Duration};
    ///
    /// use wireframe::fragment::Reassembler;
    /// let cap = NonZeroUsize::new(1024).expect("non-zero cap");
    /// let reassembler = Reassembler::new(cap, Duration::from_secs(30), Some(cap), Some(cap));
    /// assert_eq!(reassembler.buffered_bytes(), 0);
    /// ```
    #[must_use]
    pub const fn buffered_bytes(&self) -> usize { self.budget.buffered_bytes() }

    /// Append an accepted payload and remove the entry when it completes.
    fn append_and_maybe_complete(
        budget: &mut budget::ReassemblyBudget,
        mut occupied: OccupiedEntry<'_, MessageId, PartialMessage>,
        payload: &[u8],
        completes: bool,
    ) -> Result<Option<ReassembledMessage>, ReassemblyError> {
        let message_id = *occupied.key();
        let Some(attempted) = occupied.get().len().checked_add(payload.len()) else {
            let limit = budget.max_message_size();
            budget.remove_partial(occupied);
            return Err(ReassemblyError::MessageTooLarge {
                message_id,
                attempted: usize::MAX,
                limit,
            });
        };
        if let Err(err) = budget.assert_within_message_limit(message_id, attempted) {
            budget.remove_partial(occupied);
            return Err(err);
        }
        if let Err(err) = budget.add_bytes(payload.len()) {
            budget.remove_partial(occupied);
            return Err(err);
        }

        occupied.get_mut().push(payload);
        if completes {
            let partial = budget.remove_partial(occupied);
            let buffer = partial.into_buffer();
            Ok(Some(ReassembledMessage::new(message_id, buffer)))
        } else {
            Ok(None)
        }
    }

    /// Validate and append a fragment for an already-started series.
    fn push_existing_fragment(
        budget: &mut budget::ReassemblyBudget,
        mut occupied: OccupiedEntry<'_, MessageId, PartialMessage>,
        header: FragmentHeader,
        payload: &[u8],
    ) -> Result<Option<ReassembledMessage>, ReassemblyError> {
        let status = occupied
            .get_mut()
            .series
            .accept(header)
            .map_err(ReassemblyError::from);

        match status {
            Ok(FragmentStatus::Incomplete) => {
                Self::append_and_maybe_complete(budget, occupied, payload, false)
            }
            Ok(FragmentStatus::Duplicate) => Ok(None),
            Ok(FragmentStatus::Complete) => {
                Self::append_and_maybe_complete(budget, occupied, payload, true)
            }
            Err(err) => {
                budget.remove_partial(occupied);
                Err(err)
            }
        }
    }
}
