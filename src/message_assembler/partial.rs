//! In-flight message assembly state.
//!
//! [`PartialAssembly`] holds the bytes and routing metadata accumulated
//! between a first frame and the frame that completes the message. Keeping
//! this state in its own module lets [`MessageAssemblyState`] concentrate on
//! admission decisions while the completion check lives next to the data it
//! validates.
//!
//! [`MessageAssemblyState`]: super::state::MessageAssemblyState

use std::time::Instant;

use super::{
    ContinuationFrameHeader,
    MessageKey,
    error::{MessageAssemblyError, MessageSeriesError, MessageSeriesStatus},
    series::MessageSeries,
    types::{AssembledMessage, EnvelopeRouting},
    validation,
};

/// Partial message assembly in progress.
#[derive(Debug)]
pub(super) struct PartialAssembly {
    /// Ordering state that rejects gaps, duplicates, and post-completion data.
    series: MessageSeries,
    /// Envelope routing retained until the assembled message is emitted.
    routing: EnvelopeRouting,
    /// First-frame metadata kept separate from the body for reconstruction.
    metadata: Vec<u8>,
    /// Ordered body bytes accumulated from first and continuation frames.
    body: Vec<u8>,
    /// Start time used to discard incomplete peer-controlled state.
    pub(super) started_at: Instant,
}

impl PartialAssembly {
    /// Start a new assembly from an admitted first frame.
    pub(super) fn new(
        series: MessageSeries,
        routing: EnvelopeRouting,
        metadata: Vec<u8>,
        body: &[u8],
        started_at: Instant,
    ) -> Self {
        Self {
            series,
            routing,
            metadata,
            body: body.to_vec(),
            started_at,
        }
    }

    /// Body bytes buffered so far, excluding metadata.
    pub(super) const fn body_len(&self) -> usize { self.body.len() }

    /// Declared total body length, if the first frame supplied one.
    pub(super) fn declared_total(&self) -> Option<usize> { self.series.expected_total() }

    /// Validate continuity and report whether the series is now complete.
    pub(super) fn accept_continuation(
        &mut self,
        header: &ContinuationFrameHeader,
    ) -> Result<MessageSeriesStatus, MessageSeriesError> {
        self.series.accept_continuation(header)
    }

    /// Append an admitted continuation body.
    pub(super) fn push_body(&mut self, body: &[u8]) { self.body.extend_from_slice(body); }

    /// Bytes retained for this assembly, including metadata.
    pub(super) const fn buffered_bytes(&self) -> usize {
        self.metadata.len().saturating_add(self.body.len())
    }

    /// Consume the assembly, enforcing any declared total.
    ///
    /// The partial state is always freed. A mismatch is reported instead of
    /// dispatching a body whose length contradicts the peer's declaration.
    ///
    /// # Errors
    ///
    /// Returns [`MessageAssemblyError::DeclaredTotalMismatch`] when the
    /// assembled body length differs from the declared total.
    pub(super) fn into_message(self) -> Result<Option<AssembledMessage>, MessageAssemblyError> {
        let key: MessageKey = self.series.message_key();
        if let Some(declared) = self.series.expected_total() {
            validation::validate_completion(key, declared, self.body.len())?;
        }

        Ok(Some(AssembledMessage::new(
            key,
            self.routing,
            self.metadata,
            self.body,
        )))
    }
}
