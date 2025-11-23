//! Outbound helper that splits logical messages into transport fragments.
//!
//! [`Fragmenter`] exposes a small API for chunking serialized messages into
//! fixed-size fragments, tagging each fragment with a [`FragmentHeader`]. The
//! struct tracks unique [`MessageId`] values internally so callers can request
//! chunking without worrying about identifier collisions.

use std::{
    num::NonZeroUsize,
    sync::atomic::{AtomicU64, Ordering},
};

use super::{FragmentHeader, FragmentIndex, FragmentationError, MessageId};
use crate::message::Message;

/// Splits logical messages into fragment-sized frames.
#[derive(Debug)]
pub struct Fragmenter {
    max_fragment_size: NonZeroUsize,
    next_message_id: AtomicU64,
}

impl Fragmenter {
    /// Create a new fragmenter that caps fragment payloads at `max_fragment_size` bytes.
    #[must_use]
    pub const fn new(max_fragment_size: NonZeroUsize) -> Self {
        Self::with_starting_id(max_fragment_size, MessageId::new(0))
    }

    /// Create a new fragmenter starting from a specific [`MessageId`].
    #[must_use]
    pub const fn with_starting_id(max_fragment_size: NonZeroUsize, start_at: MessageId) -> Self {
        Self {
            max_fragment_size,
            next_message_id: AtomicU64::new(start_at.get()),
        }
    }

    /// Return the maximum fragment payload size in bytes.
    #[must_use]
    pub const fn max_fragment_size(&self) -> NonZeroUsize { self.max_fragment_size }

    /// Generate and return the next [`MessageId`].
    ///
    /// # Panics
    ///
    /// Panics if the identifier counter reaches `u64::MAX` and overflows. Callers
    /// should provisionally treat `MessageId` values as unique for the lifetime of
    /// the fragmenter.
    #[must_use]
    pub fn next_message_id(&self) -> MessageId {
        let previous = self
            .next_message_id
            .fetch_update(Ordering::Relaxed, Ordering::Relaxed, |current| {
                current.checked_add(1)
            })
            .unwrap_or_else(|_| panic!("message id counter exhausted"));
        MessageId::new(previous)
    }

    /// Serialize `message` and split it into fragments.
    ///
    /// # Errors
    ///
    /// Returns [`FragmentationError::Encode`] if serialization fails, or
    /// [`FragmentationError::IndexOverflow`] if the fragment index would
    /// overflow `u32`.
    pub fn fragment_message<M: Message>(
        &self,
        message: &M,
    ) -> Result<FragmentBatch, FragmentationError> {
        let bytes = message.to_bytes()?;
        self.fragment_bytes(bytes)
    }

    /// Split `payload` into fragments, generating a fresh [`MessageId`].
    ///
    /// # Errors
    ///
    /// Returns [`FragmentationError::IndexOverflow`] if more than
    /// `u32::MAX + 1` fragments are required.
    pub fn fragment_bytes(
        &self,
        payload: impl AsRef<[u8]>,
    ) -> Result<FragmentBatch, FragmentationError> {
        let message_id = self.next_message_id();
        self.fragment_with_id(message_id, payload.as_ref())
    }

    /// Split `payload` into fragments, tagging them with `message_id`.
    ///
    /// # Errors
    ///
    /// Returns [`FragmentationError::IndexOverflow`] if more than
    /// `u32::MAX + 1` fragments are required.
    pub fn fragment_with_id(
        &self,
        message_id: MessageId,
        payload: impl AsRef<[u8]>,
    ) -> Result<FragmentBatch, FragmentationError> {
        let fragments = self.build_fragments(message_id, payload.as_ref())?;
        Ok(FragmentBatch::new(message_id, fragments))
    }

    fn build_fragments(
        &self,
        message_id: MessageId,
        payload: &[u8],
    ) -> Result<Vec<FragmentFrame>, FragmentationError> {
        let max = self.max_fragment_size.get();
        if payload.is_empty() {
            let header = FragmentHeader::new(message_id, FragmentIndex::zero(), true);
            return Ok(vec![FragmentFrame::new(header, Vec::new())]);
        }

        let total = payload.len();
        let mut fragments = Vec::with_capacity(div_ceil(total, max));
        let mut index = FragmentIndex::zero();
        let mut offset = 0usize;

        while offset < total {
            let end = (offset + max).min(total);
            let is_last = end == total;
            fragments.push(FragmentFrame::new(
                FragmentHeader::new(message_id, index, is_last),
                payload[offset..end].to_vec(),
            ));

            if is_last {
                break;
            }

            offset = end;
            index = index
                .checked_increment()
                .ok_or(FragmentationError::IndexOverflow { last: index })?;
        }

        Ok(fragments)
    }
}

/// Metadata and payload for a single outbound fragment.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FragmentFrame {
    header: FragmentHeader,
    payload: Vec<u8>,
}

impl FragmentFrame {
    /// Construct a new fragment frame.
    #[must_use]
    pub fn new(header: FragmentHeader, payload: Vec<u8>) -> Self { Self { header, payload } }

    /// Return the fragment header.
    #[must_use]
    pub fn header(&self) -> &FragmentHeader { &self.header }

    /// Return the fragment payload bytes.
    #[must_use]
    pub fn payload(&self) -> &[u8] { self.payload.as_slice() }

    /// Consume the frame, returning its components.
    #[must_use]
    pub fn into_parts(self) -> (FragmentHeader, Vec<u8>) { (self.header, self.payload) }
}

/// Collection of fragments produced for a single logical message.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FragmentBatch {
    message_id: MessageId,
    fragments: Vec<FragmentFrame>,
}

impl FragmentBatch {
    fn new(message_id: MessageId, fragments: Vec<FragmentFrame>) -> Self {
        debug_assert!(!fragments.is_empty(), "fragment batches must not be empty");
        Self {
            message_id,
            fragments,
        }
    }

    /// Return the [`MessageId`] shared by all fragments.
    #[must_use]
    pub const fn message_id(&self) -> MessageId { self.message_id }

    /// Return the fragments as a slice.
    #[must_use]
    pub fn fragments(&self) -> &[FragmentFrame] { self.fragments.as_slice() }

    /// Number of fragments in the batch.
    #[expect(
        clippy::len_without_is_empty,
        reason = "batches are guaranteed non-empty"
    )]
    #[must_use]
    pub fn len(&self) -> usize { self.fragments.len() }

    /// Whether the logical message required more than one fragment.
    #[must_use]
    pub fn is_fragmented(&self) -> bool { self.len() > 1 }

    /// Consume the batch, returning all fragments.
    #[must_use]
    pub fn into_fragments(self) -> Vec<FragmentFrame> { self.fragments }
}

impl IntoIterator for FragmentBatch {
    type Item = FragmentFrame;
    type IntoIter = std::vec::IntoIter<FragmentFrame>;

    fn into_iter(self) -> Self::IntoIter { self.fragments.into_iter() }
}

fn div_ceil(numerator: usize, denominator: usize) -> usize { numerator.div_ceil(denominator) }
