//! Stateful tracker for multiple concurrent message assemblies.
//!
//! `MessageAssemblyState` manages in-flight message assemblies keyed by
//! [`MessageKey`]. It routes incoming frames to the appropriate series,
//! validates continuity, and tracks completion.

use std::{
    collections::{HashMap, hash_map::Entry},
    num::NonZeroUsize,
    time::{Duration, Instant},
};

use super::{
    ContinuationFrameHeader,
    MessageKey,
    budget::{AggregateBudgets, ContinuationLimits, check_aggregate_budgets},
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
    /// Active assemblies keyed by protocol message key.
    assemblies: HashMap<MessageKey, PartialAssembly>,
    /// Aggregate limits protecting connection and in-flight memory.
    budgets: AggregateBudgets,
}
impl MessageAssemblyState {
    /// Create a new assembly state manager.
    ///
    /// # Arguments
    ///
    /// * `max_message_size` - Maximum allowed size for a single assembled message.
    /// * `timeout` - Duration after which partial assemblies are purged.
    #[must_use]
    pub fn new(max_message_size: NonZeroUsize, timeout: Duration) -> Self {
        Self::with_budgets(max_message_size, timeout, None, None)
    }
    /// Create a new assembly state manager with optional aggregate budgets.
    ///
    /// When `connection_budget` or `in_flight_budget` is `Some`, frames that
    /// would cause the total buffered bytes across all in-flight assemblies
    /// to exceed the respective limit are rejected.
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
            assemblies: HashMap::new(),
            budgets: AggregateBudgets {
                connection: connection_budget,
                in_flight: in_flight_budget,
            },
        }
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
        if self.assemblies.contains_key(&key) {
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

        // Check aggregate budgets before buffering.  Single-frame messages
        // are returned above and never counted.  The O(n) scan is gated on
        // is_enabled() so the hot path stays O(1) when budgets are absent.
        if self.budgets.is_enabled() {
            let incoming_bytes = input.body.len().saturating_add(input.metadata.len());
            check_aggregate_budgets(
                key,
                self.total_buffered_bytes(),
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
    /// validation fails, or if the size limit would be exceeded.
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
    /// validation fails, or if the size limit would be exceeded.
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

        // Snapshot budget state before the mutable entry borrow.  The O(n)
        // scan is gated on is_enabled() so it's skipped when budgets are absent.
        let limits = ContinuationLimits {
            max_message_size: self.max_message_size,
            budgets: self.budgets,
        };
        let buffered_total = if limits.budgets.is_enabled() {
            self.total_buffered_bytes()
        } else {
            0
        };

        let Entry::Occupied(mut entry) = self.assemblies.entry(key) else {
            return Err(MessageAssemblyError::Series(
                MessageSeriesError::MissingFirstFrame { key },
            ));
        };

        // Validate continuity
        let status = match entry.get_mut().accept_continuation(header) {
            Ok(s) => s,
            Err(e) => {
                if Self::is_unrecoverable_continuity_error(&e) {
                    entry.remove();
                }
                return Err(MessageAssemblyError::Series(e));
            }
        };

        let partial = entry.get_mut();
        if let Err(e) = partial.admit_continuation(body.len(), buffered_total, limits) {
            entry.remove();
            return Err(e);
        }
        partial.push_body(body);

        let MessageSeriesStatus::Complete = status else {
            return Ok(None);
        };
        entry.remove().into_message()
    }

    /// Remove any partial assemblies that exceeded the configured timeout.
    ///
    /// Returns the keys of evicted assemblies.
    pub fn purge_expired(&mut self) -> Vec<MessageKey> { self.purge_expired_at(Instant::now()) }

    /// Remove expired assemblies using an explicit clock reading.
    ///
    /// Returns the keys of evicted assemblies.
    pub fn purge_expired_at(&mut self, now: Instant) -> Vec<MessageKey> {
        let mut evicted = Vec::new();
        let timeout = self.timeout;

        self.assemblies.retain(|key, partial| {
            let expired = now.saturating_duration_since(partial.started_at) >= timeout;
            if expired {
                evicted.push(*key);
            }
            !expired
        });

        evicted
    }

    /// Total bytes buffered across all in-flight assemblies.
    ///
    /// Includes both body and metadata bytes for each partial assembly.
    #[must_use]
    pub fn total_buffered_bytes(&self) -> usize {
        self.assemblies
            .values()
            .map(PartialAssembly::buffered_bytes)
            .sum()
    }

    /// Number of partial assemblies currently buffered.
    #[must_use]
    pub fn buffered_count(&self) -> usize { self.assemblies.len() }
}
