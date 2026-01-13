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
    error::{MessageAssemblyError, MessageSeriesError, MessageSeriesStatus},
    series::MessageSeries,
    types::{AssembledMessage, FirstFrameInput},
};

/// Partial message assembly in progress.
#[derive(Debug)]
struct PartialAssembly {
    series: MessageSeries,
    metadata: Vec<u8>,
    body_buffer: Vec<u8>,
    started_at: Instant,
}

impl PartialAssembly {
    fn new(series: MessageSeries, started_at: Instant) -> Self {
        Self {
            series,
            metadata: Vec::new(),
            body_buffer: Vec::new(),
            started_at,
        }
    }

    fn push_body(&mut self, data: &[u8]) { self.body_buffer.extend_from_slice(data); }

    fn set_metadata(&mut self, data: Vec<u8>) { self.metadata = data; }

    fn accumulated_len(&self) -> usize { self.body_buffer.len() }
}

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
/// let input =
///     FirstFrameInput::new(&first, vec![0x01, 0x02], b"hello").expect("header lengths match");
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
    max_message_size: NonZeroUsize,
    timeout: Duration,
    assemblies: HashMap<MessageKey, PartialAssembly>,
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
        Self {
            max_message_size,
            timeout,
            assemblies: HashMap::new(),
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
    /// for this key is already in progress, or [`MessageAssemblyError::MessageTooLarge`]
    /// if the body exceeds the configured limit.
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
    /// for this key is already in progress, or [`MessageAssemblyError::MessageTooLarge`]
    /// if the body exceeds the configured limit.
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

        let series = MessageSeries::from_first_frame(input.header);

        // If this is a single-frame message, return immediately
        if input.header.is_last {
            return Ok(Some(AssembledMessage::new(
                key,
                input.metadata,
                input.body.to_vec(),
            )));
        }

        // Start new assembly
        let mut partial = PartialAssembly::new(series, now);
        partial.set_metadata(input.metadata);
        partial.push_body(input.body);
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

    /// Check if accumulated size plus new body would exceed the limit.
    ///
    /// Returns the new total size on success.
    fn check_size_limit(
        max_message_size: NonZeroUsize,
        key: MessageKey,
        accumulated: usize,
        body_len: usize,
    ) -> Result<usize, MessageAssemblyError> {
        let Some(new_len) = accumulated.checked_add(body_len) else {
            return Err(MessageAssemblyError::MessageTooLarge {
                key,
                attempted: usize::MAX,
                limit: max_message_size,
            });
        };

        if new_len > max_message_size.get() {
            return Err(MessageAssemblyError::MessageTooLarge {
                key,
                attempted: new_len,
                limit: max_message_size,
            });
        }

        Ok(new_len)
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

        let Entry::Occupied(mut entry) = self.assemblies.entry(key) else {
            return Err(MessageAssemblyError::Series(
                MessageSeriesError::MissingFirstFrame { key },
            ));
        };

        // Validate continuity
        let status = match entry.get_mut().series.accept_continuation(header) {
            Ok(s) => s,
            Err(e) => {
                if Self::is_unrecoverable_continuity_error(&e) {
                    entry.remove();
                }
                return Err(MessageAssemblyError::Series(e));
            }
        };

        // Check size limit
        let accumulated = entry.get().accumulated_len();
        if let Err(e) = Self::check_size_limit(self.max_message_size, key, accumulated, body.len())
        {
            entry.remove();
            return Err(e);
        }

        entry.get_mut().push_body(body);

        match status {
            MessageSeriesStatus::Incomplete => Ok(None),
            MessageSeriesStatus::Complete => {
                let partial = entry.remove();
                Ok(Some(AssembledMessage::new(
                    key,
                    partial.metadata,
                    partial.body_buffer,
                )))
            }
        }
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

    /// Number of partial assemblies currently buffered.
    #[must_use]
    pub fn buffered_count(&self) -> usize { self.assemblies.len() }
}
