//! Stateful tracker for multiple concurrent message assemblies.
//!
//! `MessageAssemblyState` manages in-flight message assemblies keyed by
//! [`MessageKey`]. It routes incoming frames to the appropriate series,
//! validates continuity, and tracks completion.

use std::{
    num::NonZeroUsize,
    time::{Duration, Instant},
};

use super::{
    ContinuationFrameHeader,
    MessageKey,
    accounting::AssemblyTracker,
    budget::{
        AggregateBudgets,
        ContinuationLimits,
        MESSAGE_ASSEMBLY_ENTRY_OVERHEAD_BYTES,
        check_aggregate_budgets,
    },
    error::{MessageAssemblyError, MessageSeriesError, MessageSeriesStatus},
    partial::PartialAssembly,
    types::{AssembledMessage, FirstFrameInput},
    validation,
};

/// Stateful manager for multiple concurrent message assemblies.
///
/// Tracks in-flight assemblies keyed by [`MessageKey`], applying continuity
/// validation and enforcing size and timeout limits.
///
/// # Examples
///
/// ```
/// use std::{num::NonZeroUsize, time::Duration};
///
/// use wireframe::message_assembler::{
///     ContinuationFrameHeader,
///     EnvelopeId,
///     EnvelopeRouting,
///     FirstFrameHeader,
///     FirstFrameInput,
///     FrameSequence,
///     MessageAssemblyState,
///     MessageKey,
/// };
///
/// let mut state = MessageAssemblyState::new(
///     NonZeroUsize::new(1024).expect("non-zero message size"),
///     Duration::from_secs(30),
/// );
///
/// // Start assembly for key 1
/// let first = FirstFrameHeader {
///     message_key: MessageKey(1),
///     metadata_len: 2,
///     body_len: 5,
///     total_body_len: Some(10),
///     is_last: false,
/// };
/// let routing = EnvelopeRouting {
///     envelope_id: EnvelopeId(1),
///     correlation_id: None,
/// };
/// let input = FirstFrameInput::new(&first, routing, vec![0x01, 0x02], b"hello")
///     .expect("header lengths match");
/// let msg = state
///     .accept_first_frame(input)
///     .expect("first frame accepted");
/// assert!(msg.is_none()); // Not yet complete
///
/// // Complete with continuation
/// let cont = ContinuationFrameHeader {
///     message_key: MessageKey(1),
///     sequence: Some(FrameSequence(1)),
///     body_len: 5,
///     is_last: true,
/// };
/// let msg = state
///     .accept_continuation_frame(&cont, b"world")
///     .expect("continuation accepted")
///     .expect("message should complete");
/// assert_eq!(msg.body(), b"helloworld");
/// ```
#[derive(Debug)]
pub struct MessageAssemblyState {
    /// Maximum body size permitted for one logical message.
    max_message_size: NonZeroUsize,
    /// Age after which incomplete assemblies are removed.
    timeout: Duration,
    /// Active assemblies with their running byte total and deadline index.
    assemblies: AssemblyTracker,
    /// Optional cap on the number of concurrently in-flight assemblies.
    max_concurrent_assemblies: Option<NonZeroUsize>,
    /// Aggregate limits protecting connection and in-flight memory.
    budgets: AggregateBudgets,
}
impl MessageAssemblyState {
    /// Create a new assembly state manager.
    ///
    /// `max_message_size` bounds one assembled message, and `timeout` is the age
    /// after which partial assemblies are purged.
    #[must_use]
    pub fn new(max_message_size: NonZeroUsize, timeout: Duration) -> Self {
        Self::with_budgets(max_message_size, timeout, None, None)
    }
    /// Create a new assembly state manager with optional aggregate budgets.
    ///
    /// Charged bytes — retained payload plus a fixed per-entry charge — are
    /// compared against whichever limits are configured, so an empty assembly
    /// still consumes budget.
    #[must_use]
    pub fn with_budgets(
        max_message_size: NonZeroUsize,
        timeout: Duration,
        connection_budget: Option<NonZeroUsize>,
        in_flight_budget: Option<NonZeroUsize>,
    ) -> Self {
        Self {
            max_message_size,
            timeout,
            assemblies: AssemblyTracker::new(),
            max_concurrent_assemblies: None,
            budgets: AggregateBudgets {
                connection: connection_budget,
                in_flight: in_flight_budget,
            },
        }
    }

    /// Cap the number of assemblies that may be in flight at once.
    ///
    /// Only a new first frame is capped; continuations are unaffected. `None`
    /// leaves the count uncapped, the default for [`Self::new`] and
    /// [`Self::with_budgets`].
    #[must_use]
    pub const fn with_max_concurrent_assemblies(mut self, limit: Option<NonZeroUsize>) -> Self {
        self.max_concurrent_assemblies = limit;
        self
    }
    /// Process a first frame, starting a new assembly.
    ///
    /// Returns `Ok(Some(msg))` if the first frame is also the last (single-
    /// frame message), `Ok(None)` if assembly is in progress, or an error if
    /// the key already has an active assembly or the body exceeds the size
    /// limit.
    ///
    /// # Errors
    ///
    /// Returns [`MessageAssemblyError::DuplicateFirstFrame`] if an assembly
    /// for this key is already in progress,
    /// [`MessageAssemblyError::MessageTooLarge`] if the body exceeds the
    /// configured limit, or a declared-total error when the declaration is
    /// unsatisfiable or not honoured.
    pub fn accept_first_frame(
        &mut self,
        input: FirstFrameInput<'_>,
    ) -> Result<Option<AssembledMessage>, MessageAssemblyError> {
        self.accept_first_frame_at(input, Instant::now())
    }
    /// Process a first frame with an explicit timestamp.
    ///
    /// See [`accept_first_frame`](Self::accept_first_frame) for details.
    ///
    /// # Errors
    ///
    /// Returns [`MessageAssemblyError::DuplicateFirstFrame`] if an assembly
    /// for this key is already in progress,
    /// [`MessageAssemblyError::MessageTooLarge`] if the body exceeds the
    /// configured limit, or a declared-total error when the declaration is
    /// unsatisfiable or not honoured.
    pub fn accept_first_frame_at(
        &mut self,
        input: FirstFrameInput<'_>,
        now: Instant,
    ) -> Result<Option<AssembledMessage>, MessageAssemblyError> {
        self.purge_expired_at(now);
        let key = input.header.message_key;

        // Check for duplicate first frame
        if self.assemblies.contains_key(key) {
            return Err(MessageAssemblyError::DuplicateFirstFrame { key });
        }

        // A declared total is a protocol commitment: reject declarations that
        // the first frame already contradicts before consulting size limits.
        validation::validate_first_frame(input.header, input.body.len())?;

        // Validate message size (prefer declared total body length, include metadata)
        let declared_body_len = input.header.total_body_len.unwrap_or(input.body.len());
        let total_message_size = declared_body_len.saturating_add(input.metadata.len());

        if total_message_size > self.max_message_size.get() {
            return Err(MessageAssemblyError::MessageTooLarge {
                key,
                attempted: total_message_size,
                limit: self.max_message_size,
            });
        }

        // If this is a single-frame message, return immediately
        if input.header.is_last {
            return Ok(Some(AssembledMessage::new(
                key,
                input.routing,
                input.metadata,
                input.body.to_vec(),
            )));
        }

        // Enforce the concurrent-assembly cap before charging or retaining
        // anything. A duplicate key was rejected above, so this only ever
        // counts a genuinely new assembly; single-frame messages returned
        // above never enter the map and bypass the cap.
        if let Some(limit) = self.max_concurrent_assemblies {
            let attempted = self.buffered_count().saturating_add(1);
            if attempted > limit.get() {
                return Err(MessageAssemblyError::TooManyConcurrentAssemblies {
                    key,
                    attempted,
                    limit,
                });
            }
        }

        // Check aggregate budgets before buffering.  Single-frame messages
        // are returned above and never counted.  The running total makes this
        // O(1); the gate keeps the arithmetic off a path with no budgets.
        if self.budgets.is_enabled() {
            let incoming_bytes = input
                .body
                .len()
                .saturating_add(input.metadata.len())
                .saturating_add(MESSAGE_ASSEMBLY_ENTRY_OVERHEAD_BYTES);
            check_aggregate_budgets(
                key,
                self.total_charged_bytes(),
                incoming_bytes,
                &self.budgets,
            )?;
        }

        // Start new assembly, preserving envelope routing metadata from the
        // first frame so the completed message is dispatched correctly.
        let partial = PartialAssembly::from_first_frame(input, now);
        self.assemblies.insert(key, partial);

        Ok(None)
    }
    /// Process a continuation frame.
    ///
    /// Returns `Ok(Some(msg))` if the message is now complete, `Ok(None)` if
    /// more frames are expected, or an error if validation fails.
    ///
    /// # Errors
    ///
    /// Returns an error if no assembly exists for this key, if continuity
    /// validation fails, or if the size limit would be exceeded. Returns
    /// [`MessageAssemblyError::DeclaredTotalExceeded`] when the frame would
    /// carry the body past its declared total, or
    /// [`MessageAssemblyError::DeclaredTotalMismatch`] when the completing
    /// frame leaves the body short of the declaration.
    pub fn accept_continuation_frame(
        &mut self,
        header: &ContinuationFrameHeader,
        body: &[u8],
    ) -> Result<Option<AssembledMessage>, MessageAssemblyError> {
        self.accept_continuation_frame_at(header, body, Instant::now())
    }
    /// Whether a continuity error is unrecoverable and requires assembly removal.
    ///
    /// Unrecoverable errors (`KeyMismatch`, `SequenceOverflow`, etc.) indicate the
    /// assembly is corrupted. Recoverable errors (`DuplicateFrame`, `SequenceMismatch`,
    /// `SeriesComplete`) may be transient and the assembly is retained for recovery
    /// or timeout-based cleanup.
    const fn is_unrecoverable_continuity_error(error: &MessageSeriesError) -> bool {
        matches!(
            error,
            MessageSeriesError::KeyMismatch { .. }
                | MessageSeriesError::SequenceOverflow { .. }
                | MessageSeriesError::MissingFirstFrame { .. }
                | MessageSeriesError::MissingSequence { .. }
                | MessageSeriesError::ContinuationBodyLengthMismatch { .. }
        )
    }

    /// Process a continuation frame with an explicit timestamp.
    ///
    /// See [`accept_continuation_frame`](Self::accept_continuation_frame) for
    /// details.
    ///
    /// # Errors
    ///
    /// Returns an error if no assembly exists for this key, if continuity
    /// validation fails, or if the size limit would be exceeded. Returns
    /// [`MessageAssemblyError::DeclaredTotalExceeded`] when the frame would
    /// carry the body past its declared total, or
    /// [`MessageAssemblyError::DeclaredTotalMismatch`] when the completing
    /// frame leaves the body short of the declaration.
    pub fn accept_continuation_frame_at(
        &mut self,
        header: &ContinuationFrameHeader,
        body: &[u8],
        now: Instant,
    ) -> Result<Option<AssembledMessage>, MessageAssemblyError> {
        self.purge_expired_at(now);

        let key = header.message_key;

        // Validate header body_len matches actual payload
        if header.body_len != body.len() {
            return Err(MessageAssemblyError::Series(
                MessageSeriesError::ContinuationBodyLengthMismatch {
                    key,
                    header_len: header.body_len,
                    actual_len: body.len(),
                },
            ));
        }

        // Snapshot budget state before the mutable borrow.  The running total
        // makes this O(1); the gate keeps the arithmetic off a path with no
        // budgets.
        let limits = ContinuationLimits {
            max_message_size: self.max_message_size,
            budgets: self.budgets,
        };
        let buffered_total = if limits.budgets.is_enabled() {
            self.total_charged_bytes()
        } else {
            0
        };

        let Some(partial) = self.assemblies.get_mut(key) else {
            return Err(MessageAssemblyError::Series(
                MessageSeriesError::MissingFirstFrame { key },
            ));
        };

        // Validate continuity
        let status = match partial.accept_continuation(header) {
            Ok(s) => s,
            Err(e) => {
                if Self::is_unrecoverable_continuity_error(&e) {
                    self.assemblies.take(key);
                }
                return Err(MessageAssemblyError::Series(e));
            }
        };

        if let Err(e) = partial.admit_continuation(body.len(), buffered_total, limits) {
            self.assemblies.take(key);
            return Err(e);
        }
        self.assemblies.append_body(key, body);

        let MessageSeriesStatus::Complete = status else {
            return Ok(None);
        };
        self.assemblies
            .take(key)
            .map_or(Ok(None), PartialAssembly::into_message)
    }

    /// Remove any partial assemblies that exceeded the configured timeout.
    ///
    /// Returns the keys of evicted assemblies.
    pub fn purge_expired(&mut self) -> Vec<MessageKey> { self.purge_expired_at(Instant::now()) }

    /// Remove expired assemblies using an explicit clock reading.
    ///
    /// Only assemblies whose deadlines have passed are visited, so the cost is
    /// proportional to the number evicted rather than the number buffered.
    /// Returns the keys of evicted assemblies.
    pub fn purge_expired_at(&mut self, now: Instant) -> Vec<MessageKey> {
        self.assemblies.purge_expired_at(now, self.timeout)
    }

    /// Total payload and metadata bytes retained across all in-flight assemblies.
    ///
    /// Read from the running total, never recomputed by scanning.
    #[must_use]
    pub const fn total_buffered_bytes(&self) -> usize { self.assemblies.total_buffered_bytes() }

    /// Bytes charged against the aggregate budgets.
    ///
    /// [`Self::total_buffered_bytes`] plus a fixed per-entry charge, so an
    /// assembly carrying no payload is not free.
    #[must_use]
    pub fn total_charged_bytes(&self) -> usize { self.assemblies.total_charged_bytes() }

    /// Number of partial assemblies currently buffered.
    #[must_use]
    pub fn buffered_count(&self) -> usize { self.assemblies.len() }
}

#[cfg(test)]
#[path = "state_test_support.rs"]
mod test_support;
