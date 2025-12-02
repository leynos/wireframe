//! Inbound helper that stitches fragments back into complete messages.
//!
//! [`Reassembler`] mirrors the outbound [`Fragmenter`](crate::fragment::Fragmenter) by
//! collecting fragment payloads keyed by [`MessageId`](crate::fragment::MessageId).
//! It enforces ordering via [`FragmentSeries`](crate::fragment::FragmentSeries), guards
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

#[derive(Debug)]
struct PartialMessage {
    series: FragmentSeries,
    buffer: Vec<u8>,
    started_at: Instant,
}

impl PartialMessage {
    fn new(series: FragmentSeries, payload: &[u8], started_at: Instant) -> Self {
        Self {
            series,
            buffer: payload.to_vec(),
            started_at,
        }
    }

    fn push(&mut self, payload: &[u8]) { self.buffer.extend_from_slice(payload); }

    fn len(&self) -> usize { self.buffer.len() }

    fn started_at(&self) -> Instant { self.started_at }

    fn into_buffer(self) -> Vec<u8> { self.buffer }
}

/// Container for a fully re-assembled message payload.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ReassembledMessage {
    message_id: MessageId,
    payload: Vec<u8>,
}

impl ReassembledMessage {
    /// Construct a new [`ReassembledMessage`].
    #[must_use]
    pub fn new(message_id: MessageId, payload: Vec<u8>) -> Self {
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
    pub fn payload(&self) -> &[u8] { self.payload.as_slice() }

    /// Consume the message, returning the owned payload bytes.
    #[must_use]
    pub fn into_payload(self) -> Vec<u8> { self.payload }

    /// Decode the payload into a strongly typed message.
    ///
    /// # Errors
    ///
    /// Returns any [`DecodeError`] raised while deserialising the payload.
    pub fn decode<M: Message>(&self) -> Result<M, DecodeError> {
        let (message, _) = M::from_bytes(self.payload())?;
        Ok(message)
    }
}

/// Stateful fragment re-assembler with timeout-based eviction.
#[derive(Debug)]
pub struct Reassembler {
    max_message_size: NonZeroUsize,
    timeout: Duration,
    buffers: HashMap<MessageId, PartialMessage>,
}

impl Reassembler {
    /// Create a new re-assembler that enforces a maximum reconstructed payload size.
    #[must_use]
    pub fn new(max_message_size: NonZeroUsize, timeout: Duration) -> Self {
        Self {
            max_message_size,
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

        let payload = payload.as_ref();

        match self.buffers.entry(header.message_id()) {
            Entry::Occupied(mut occupied) => {
                let status = occupied
                    .get_mut()
                    .series
                    .accept(header)
                    .map_err(ReassemblyError::from);

                match status {
                    Ok(FragmentStatus::Incomplete) => Self::append_and_maybe_complete(
                        self.max_message_size,
                        occupied,
                        payload,
                        false,
                    ),
                    Ok(FragmentStatus::Complete) => Self::append_and_maybe_complete(
                        self.max_message_size,
                        occupied,
                        payload,
                        true,
                    ),
                    Err(err) => {
                        occupied.remove();
                        Err(err)
                    }
                }
            }
            Entry::Vacant(vacant) => {
                let mut series = FragmentSeries::new(header.message_id());
                let status = series.accept(header).map_err(ReassemblyError::from)?;
                let attempted = payload.len();
                Self::assert_within_limit(self.max_message_size, header.message_id(), attempted)?;

                match status {
                    FragmentStatus::Incomplete => {
                        vacant.insert(PartialMessage::new(series, payload, now));
                        Ok(None)
                    }
                    FragmentStatus::Complete => Ok(Some(ReassembledMessage::new(
                        header.message_id(),
                        payload.to_vec(),
                    ))),
                }
            }
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

        self.buffers.retain(|message_id, partial| {
            let expired = now.saturating_duration_since(partial.started_at()) >= timeout;
            if expired {
                evicted.push(*message_id);
            }
            !expired
        });

        evicted
    }

    /// Number of partial messages currently buffered.
    #[must_use]
    pub fn buffered_len(&self) -> usize { self.buffers.len() }

    fn assert_within_limit(
        limit: NonZeroUsize,
        message_id: MessageId,
        attempted: usize,
    ) -> Result<(), ReassemblyError> {
        if attempted > limit.get() {
            return Err(ReassemblyError::MessageTooLarge {
                message_id,
                attempted,
                limit,
            });
        }
        Ok(())
    }

    fn append_and_maybe_complete(
        limit: NonZeroUsize,
        mut occupied: OccupiedEntry<'_, MessageId, PartialMessage>,
        payload: &[u8],
        completes: bool,
    ) -> Result<Option<ReassembledMessage>, ReassemblyError> {
        let message_id = *occupied.key();
        let Some(attempted) = occupied.get().len().checked_add(payload.len()) else {
            occupied.remove();
            return Err(ReassemblyError::MessageTooLarge {
                message_id,
                attempted: usize::MAX,
                limit,
            });
        };
        if let Err(err) = Self::assert_within_limit(limit, message_id, attempted) {
            occupied.remove();
            return Err(err);
        }

        occupied.get_mut().push(payload);
        if completes {
            let buffer = occupied.remove().into_buffer();
            Ok(Some(ReassembledMessage::new(message_id, buffer)))
        } else {
            Ok(None)
        }
    }
}
