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
    budget::{ContinuationLimits, check_aggregate_budgets, check_size_limit},
    error::{MessageAssemblyError, MessageSeriesError, MessageSeriesStatus},
    series::MessageSeries,
    types::{AssembledMessage, EnvelopeRouting, FirstFrameInput},
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
    body_buffer: Vec<u8>,
    /// Start time used to discard incomplete peer-controlled state.
    pub(super) started_at: Instant,
}

impl PartialAssembly {
    /// Start a new assembly from an admitted first frame.
    ///
    /// The input's header has already been validated against its payloads and
    /// its declared total, so this only stores the components.
    pub(super) fn from_first_frame(input: FirstFrameInput<'_>, started_at: Instant) -> Self {
        let series = MessageSeries::from_first_frame(input.header);
        Self {
            series,
            routing: input.routing,
            metadata: input.metadata,
            body_buffer: input.body.to_vec(),
            started_at,
        }
    }

    /// Body bytes buffered so far, excluding metadata.
    pub(super) fn body_len(&self) -> usize { self.body_buffer.len() }

    /// Declared total body length, if the first frame supplied one.
    pub(super) fn declared_total(&self) -> Option<usize> { self.series.expected_total() }

    /// Validate continuity and report whether the series is now complete.
    pub(super) fn accept_continuation(
        &mut self,
        header: &ContinuationFrameHeader,
    ) -> Result<MessageSeriesStatus, MessageSeriesError> {
        self.series.accept_continuation(header)
    }

    /// Apply size, budget, and declared-total admission checks to a
    /// continuation body before it is buffered.
    ///
    /// # Errors
    ///
    /// Returns [`MessageAssemblyError::MessageTooLarge`] or an aggregate
    /// budget error when the frame cannot be retained, and
    /// [`MessageAssemblyError::DeclaredTotalExceeded`] when it would push the
    /// accumulated body past the declared total. The caller frees the
    /// assembly on any error.
    pub(super) fn admit_continuation(
        &self,
        body_len: usize,
        buffered_total: usize,
        limits: ContinuationLimits,
    ) -> Result<(), MessageAssemblyError> {
        let key = self.series.message_key();
        let accumulated = self.body_len();
        check_size_limit(limits.max_message_size, key, accumulated, body_len)?;
        check_aggregate_budgets(key, buffered_total, body_len, &limits.budgets)?;

        // Enforce the declared total before buffering so an oversized
        // continuation cannot be retained even briefly.
        if let Some(declared) = self.declared_total() {
            validation::validate_continuation(key, declared, accumulated, body_len)?;
        }
        Ok(())
    }

    /// Append an admitted continuation body.
    pub(super) fn push_body(&mut self, body: &[u8]) { self.body_buffer.extend_from_slice(body); }

    /// Bytes retained for this assembly, including metadata.
    pub(super) fn buffered_bytes(&self) -> usize {
        self.metadata.len().saturating_add(self.body_buffer.len())
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
            validation::validate_completion(key, declared, self.body_buffer.len())?;
        }

        Ok(Some(AssembledMessage::new(
            key,
            self.routing,
            self.metadata,
            self.body_buffer,
        )))
    }
}
