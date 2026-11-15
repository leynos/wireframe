//! Streaming message assembly state.
//!
//! A streaming route consumes a logical message while its frames are still
//! arriving. The assembler therefore retains only the ordering state needed to
//! validate the series: the message key, the envelope routing captured from the
//! first frame, the number of body bytes admitted so far, and any declared
//! total. Metadata and body bytes are handed to the handler immediately and are
//! never buffered here.
//!
//! This separation is what keeps a streaming message out of the aggregate
//! assembly budgets: those budgets account for bytes the connection retains,
//! while delivered chunks are accounted by the request-body channel until the
//! consumer takes them.

use std::time::Instant;

use super::{
    ContinuationFrameHeader,
    MessageAssemblyError,
    MessageKey,
    MessageSeriesError,
    MessageSeriesStatus,
    budget::check_size_limit,
    series::MessageSeries,
    types::{EnvelopeRouting, FirstFrameInput},
    validation,
};

/// A first frame admitted for immediate delivery to a streaming route.
///
/// The body slice borrows the frame payload; callers copy it into an owned
/// request-body chunk before the frame is released.
#[derive(Debug)]
pub struct StreamingFirstFrame<'a> {
    /// Key correlating the remaining frames of this message.
    pub(super) key: MessageKey,
    /// Envelope routing captured from the first frame.
    pub(super) routing: EnvelopeRouting,
    /// Protocol metadata carried by the first frame.
    pub(super) metadata: Vec<u8>,
    /// First body chunk, borrowed from the frame payload.
    pub(super) chunk: &'a [u8],
    /// Whether this frame also completes the message.
    pub(super) is_last: bool,
}

impl<'a> StreamingFirstFrame<'a> {
    /// Return the message key.
    #[must_use]
    pub const fn key(&self) -> MessageKey { self.key }

    /// Return the routing metadata captured from the first frame.
    #[must_use]
    pub const fn routing(&self) -> EnvelopeRouting { self.routing }

    /// Consume the outcome and return the first-frame metadata.
    #[must_use]
    pub fn into_metadata(self) -> Vec<u8> { self.metadata }

    /// Borrow the first body chunk.
    #[must_use]
    pub const fn chunk(&self) -> &'a [u8] { self.chunk }

    /// Return whether the first frame also completes the message.
    #[must_use]
    pub const fn is_last(&self) -> bool { self.is_last }
}

/// Outcome of admitting a continuation frame for a streaming message.
#[derive(Debug)]
pub enum StreamingContinuation<'a> {
    /// Deliver the borrowed body slice to the live handler.
    Chunk {
        /// Key of the message that owns this chunk.
        key: MessageKey,
        /// Body bytes carried by the frame.
        chunk: &'a [u8],
        /// Whether this frame completes the message.
        is_last: bool,
    },
    /// The message is in discard mode; the frame was validated and dropped.
    Discarded {
        /// Key of the message that was discarded.
        key: MessageKey,
    },
}

impl<'a> StreamingContinuation<'a> {
    /// Return the message key that owns this outcome.
    #[must_use]
    pub const fn key(&self) -> MessageKey {
        match self {
            Self::Chunk { key, .. } | Self::Discarded { key } => *key,
        }
    }

    /// Return whether the outcome carries a deliverable chunk.
    #[must_use]
    pub const fn is_chunk(&self) -> bool { matches!(self, Self::Chunk { .. }) }
}

/// Failure admitting a streaming continuation.
///
/// `terminal` distinguishes an unrecoverable error (the assembly was removed)
/// from a recoverable continuity error whose partial state is retained for a
/// retransmission. Only terminal failures close the handler's body stream.
#[derive(Debug, thiserror::Error)]
#[error("{source}")]
pub struct StreamingFailure {
    /// Message key whose continuation was rejected.
    key: MessageKey,
    /// Underlying assembly error.
    source: MessageAssemblyError,
    /// Whether the assembly was removed and the stream must be closed.
    terminal: bool,
}

impl StreamingFailure {
    /// Create a failure with the given terminal disposition.
    pub(super) const fn new(key: MessageKey, source: MessageAssemblyError, terminal: bool) -> Self {
        Self {
            key,
            source,
            terminal,
        }
    }

    /// Return the message key whose continuation was rejected.
    #[must_use]
    pub const fn key(&self) -> MessageKey { self.key }

    /// Return whether the failure requires closing the handler's stream.
    #[must_use]
    pub const fn is_terminal(&self) -> bool { self.terminal }

    /// Borrow the underlying assembly error.
    #[must_use]
    pub const fn source_error(&self) -> &MessageAssemblyError { &self.source }
}

/// In-flight state for a message delivered to a streaming route.
#[derive(Debug)]
pub(super) struct StreamingAssembly {
    /// Ordering state that rejects gaps, duplicates, and post-completion data.
    series: MessageSeries,
    /// Body bytes admitted so far. Metadata is not counted because it is
    /// delivered once and not retained.
    accumulated: usize,
    /// Start time used to expire abandoned peer-controlled state.
    pub(super) started_at: Instant,
    /// Whether the consumer has abandoned the message.
    discard: bool,
}

impl StreamingAssembly {
    /// Start tracking a multi-frame streaming message.
    pub(super) fn from_first_frame(
        input: &FirstFrameInput<'_>,
        chunk_len: usize,
        started_at: Instant,
    ) -> Self {
        Self {
            series: MessageSeries::from_first_frame(input.header),
            accumulated: chunk_len,
            started_at,
            discard: false,
        }
    }

    /// Return whether the final frame has been accepted.
    pub(super) const fn is_complete(&self) -> bool { self.series.is_complete() }

    /// Switch the message to discard mode.
    pub(super) const fn begin_discard(&mut self) { self.discard = true; }

    /// Validate an incoming continuation and apply size and declared-total
    /// checks before any bytes are delivered.
    ///
    /// # Errors
    ///
    /// Returns a [`StreamingFailure`] when continuity, the per-message cap, the
    /// declared total, or exact completion fails. A recoverable continuity
    /// error retains the assembly; every other failure removes it.
    pub(super) fn accept_continuation<'a>(
        &mut self,
        header: &ContinuationFrameHeader,
        body: &'a [u8],
        max_message_size: std::num::NonZeroUsize,
    ) -> Result<Option<StreamingChunk<'a>>, StreamingFailure> {
        let key = self.series.message_key();
        let status = match self.series.accept_continuation(header) {
            Ok(status) => status,
            Err(error) => return Err(self.continuity_failure(error)),
        };

        if let Err(error) = check_size_limit(max_message_size, key, self.accumulated, body.len()) {
            return Err(StreamingFailure::new(key, error, true));
        }

        if let Some(declared) = self.series.expected_total()
            && let Err(error) =
                validation::validate_continuation(key, declared, self.accumulated, body.len())
        {
            return Err(StreamingFailure::new(key, error, true));
        }

        self.accumulated = self.accumulated.saturating_add(body.len());

        let is_last = matches!(status, MessageSeriesStatus::Complete);
        if is_last && let Some(declared) = self.series.expected_total() {
            if let Err(error) = validation::validate_completion(key, declared, self.accumulated) {
                return Err(StreamingFailure::new(key, error, true));
            }
        }

        if self.discard {
            return Ok(None);
        }

        Ok(Some(StreamingChunk { body, is_last }))
    }

    /// Classify a continuity error and decide whether the assembly survives.
    fn continuity_failure(&self, error: MessageSeriesError) -> StreamingFailure {
        let key = self.series.message_key();
        let terminal = !is_recoverable_continuity_error(&error);
        StreamingFailure::new(key, MessageAssemblyError::Series(error), terminal)
    }
}

/// A validated continuation body ready for delivery.
#[derive(Debug)]
pub(super) struct StreamingChunk<'a> {
    /// Borrowed body bytes from the inbound frame.
    pub(super) body: &'a [u8],
    /// Whether this chunk completes the message.
    pub(super) is_last: bool,
}

/// Whether a continuity error leaves the assembly usable.
///
/// Duplicates and gaps can be corrected by a retransmission, so the partial is
/// retained. Key mismatches, overflows, and missing sequence data indicate a
/// corrupted series and force removal.
pub(super) const fn is_recoverable_continuity_error(error: &MessageSeriesError) -> bool {
    matches!(
        error,
        MessageSeriesError::DuplicateFrame { .. } | MessageSeriesError::SequenceMismatch { .. }
    )
}
