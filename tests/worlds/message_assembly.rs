//! Test world for message assembly multiplexing and continuity validation.
#![cfg(not(loom))]

use std::{
    collections::VecDeque,
    fmt,
    num::NonZeroUsize,
    time::{Duration, Instant},
};

use cucumber::World;
use wireframe::message_assembler::{
    AssembledMessage,
    ContinuationFrameHeader,
    FirstFrameHeader,
    FirstFrameInput,
    FrameSequence,
    MessageAssemblyError,
    MessageAssemblyState,
    MessageKey,
    MessageSeriesError,
};

use super::TestResult;

/// Parameters for creating a first frame.
#[derive(Debug)]
pub struct FirstFrameParams {
    /// Message key.
    pub key: MessageKey,
    /// Metadata bytes.
    pub metadata: Vec<u8>,
    /// Body bytes.
    pub body: Vec<u8>,
    /// Whether this is the final frame.
    pub is_last: bool,
}

impl FirstFrameParams {
    /// Create parameters for a first frame with default values.
    #[must_use]
    pub fn new(key: MessageKey, body: Vec<u8>) -> Self {
        Self {
            key,
            metadata: vec![],
            body,
            is_last: false,
        }
    }

    /// Set metadata bytes.
    #[must_use]
    pub fn with_metadata(mut self, metadata: Vec<u8>) -> Self {
        self.metadata = metadata;
        self
    }

    /// Mark as the final frame.
    #[must_use]
    pub fn final_frame(mut self) -> Self {
        self.is_last = true;
        self
    }
}

/// Parameters for creating a continuation frame.
#[derive(Debug)]
pub struct ContinuationFrameParams {
    /// Message key.
    pub key: MessageKey,
    /// Optional sequence number.
    pub sequence: Option<FrameSequence>,
    /// Body bytes.
    pub body: Vec<u8>,
    /// Whether this is the final frame.
    pub is_last: bool,
}

impl ContinuationFrameParams {
    /// Create parameters for a continuation frame with default sequence 1.
    #[must_use]
    pub fn new(key: MessageKey, body: Vec<u8>) -> Self {
        Self {
            key,
            sequence: Some(FrameSequence(1)),
            body,
            is_last: false,
        }
    }

    /// Set the sequence number.
    #[must_use]
    pub fn with_sequence(mut self, sequence: FrameSequence) -> Self {
        self.sequence = Some(sequence);
        self
    }

    /// Mark as the final frame.
    #[must_use]
    pub fn final_frame(mut self) -> Self {
        self.is_last = true;
        self
    }
}

/// Cucumber world for message assembly tests.
#[derive(Default, World)]
#[world(init = Self::new)]
pub struct MessageAssemblyWorld {
    state: Option<MessageAssemblyState>,
    current_time: Option<Instant>,
    pending_first_frames: VecDeque<PendingFirstFrame>,
    last_result: Option<Result<Option<AssembledMessage>, MessageAssemblyError>>,
    completed_messages: Vec<AssembledMessage>,
    evicted_keys: Vec<MessageKey>,
}

/// Pending first frame awaiting acceptance.
pub struct PendingFirstFrame {
    /// Frame header.
    pub header: FirstFrameHeader,
    /// Metadata bytes.
    pub metadata: Vec<u8>,
    /// Body bytes.
    pub body: Vec<u8>,
}

impl fmt::Debug for MessageAssemblyWorld {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("MessageAssemblyWorld")
            .field(
                "state",
                &self.state.as_ref().map(|_| "MessageAssemblyState"),
            )
            .field("current_time", &self.current_time)
            .field("pending_first_frames", &self.pending_first_frames.len())
            .field("last_result", &self.last_result)
            .field("completed_messages", &self.completed_messages.len())
            .field("evicted_keys", &self.evicted_keys)
            .finish()
    }
}

impl MessageAssemblyWorld {
    fn new() -> Self {
        Self {
            state: None,
            current_time: None,
            pending_first_frames: VecDeque::new(),
            last_result: None,
            completed_messages: Vec::new(),
            evicted_keys: Vec::new(),
        }
    }

    /// Initialise the assembly state with size limit and timeout.
    ///
    /// # Panics
    ///
    /// Panics if `max_size` is zero.
    pub fn create_state(&mut self, max_size: usize, timeout_secs: u64) {
        let Some(size) = NonZeroUsize::new(max_size) else {
            panic!("max_size must be non-zero");
        };
        self.state = Some(MessageAssemblyState::new(
            size,
            Duration::from_secs(timeout_secs),
        ));
        self.current_time = Some(Instant::now());
        self.pending_first_frames.clear();
        self.last_result = None;
        self.completed_messages.clear();
        self.evicted_keys.clear();
    }

    /// Queue a first frame for later acceptance (FIFO).
    pub fn add_first_frame(&mut self, params: FirstFrameParams) {
        self.pending_first_frames.push_back(PendingFirstFrame {
            header: FirstFrameHeader {
                message_key: params.key,
                metadata_len: params.metadata.len(),
                body_len: params.body.len(),
                total_body_len: None,
                is_last: params.is_last,
            },
            metadata: params.metadata,
            body: params.body,
        });
    }

    /// Accept the first queued first frame (FIFO).
    ///
    /// # Errors
    ///
    /// Returns an error if no pending frames, state not initialised, or time not set.
    pub fn accept_first_frame(&mut self) -> TestResult {
        let Some(pending) = self.pending_first_frames.pop_front() else {
            return Err("no pending first frame".into());
        };
        let Some(state) = self.state.as_mut() else {
            return Err("state not initialised".into());
        };
        let Some(now) = self.current_time else {
            return Err("time not set".into());
        };
        let input = FirstFrameInput::new(&pending.header, pending.metadata, &pending.body)
            .map_err(|e| format!("invalid input: {e}"))?;
        self.last_result = Some(state.accept_first_frame_at(input, now));
        if let Some(Ok(Some(msg))) = &self.last_result {
            self.completed_messages.push(msg.clone());
        }
        Ok(())
    }

    /// Accept all queued first frames.
    ///
    /// # Errors
    ///
    /// Returns an error if state not initialised or time not set.
    pub fn accept_all_first_frames(&mut self) -> TestResult {
        let Some(state) = self.state.as_mut() else {
            return Err("state not initialised".into());
        };
        let Some(now) = self.current_time else {
            return Err("time not set".into());
        };

        while let Some(pending) = self.pending_first_frames.pop_front() {
            let input = FirstFrameInput::new(&pending.header, pending.metadata, &pending.body)
                .map_err(|e| format!("invalid input: {e}"))?;
            let result = state.accept_first_frame_at(input, now);
            if let Ok(Some(msg)) = &result {
                self.completed_messages.push(msg.clone());
            }
            self.last_result = Some(result);
        }
        Ok(())
    }

    /// Accept a continuation frame for the given key.
    ///
    /// # Errors
    ///
    /// Returns an error if state not initialised or time not set.
    #[expect(
        clippy::needless_pass_by_value,
        reason = "parameter object consistency with add_first_frame API"
    )]
    pub fn accept_continuation(&mut self, params: ContinuationFrameParams) -> TestResult {
        let Some(state) = self.state.as_mut() else {
            return Err("state not initialised".into());
        };
        let Some(now) = self.current_time else {
            return Err("time not set".into());
        };

        let header = ContinuationFrameHeader {
            message_key: params.key,
            sequence: params.sequence,
            body_len: params.body.len(),
            is_last: params.is_last,
        };
        self.last_result = Some(state.accept_continuation_frame_at(&header, &params.body, now));
        if let Some(Ok(Some(msg))) = &self.last_result {
            self.completed_messages.push(msg.clone());
        }
        Ok(())
    }

    /// Advance the simulated clock.
    pub fn advance_time(&mut self, secs: u64) {
        if let Some(current) = self.current_time {
            self.current_time = Some(current + Duration::from_secs(secs));
        }
    }

    /// Purge expired assemblies and record evicted keys.
    ///
    /// # Errors
    ///
    /// Returns an error if state not initialised or time not set.
    pub fn purge_expired(&mut self) -> TestResult {
        let Some(state) = self.state.as_mut() else {
            return Err("state not initialised".into());
        };
        let Some(now) = self.current_time else {
            return Err("time not set".into());
        };
        self.evicted_keys = state.purge_expired_at(now);
        Ok(())
    }

    /// Number of partial assemblies currently buffered.
    #[must_use]
    pub fn buffered_count(&self) -> usize {
        self.state
            .as_ref()
            .map_or(0, MessageAssemblyState::buffered_count)
    }

    /// Whether the last result indicates an incomplete assembly.
    #[must_use]
    pub fn last_result_is_incomplete(&self) -> bool { matches!(self.last_result, Some(Ok(None))) }

    /// Body of the most recently completed message.
    #[must_use]
    pub fn last_completed_body(&self) -> Option<&[u8]> {
        self.completed_messages.last().map(AssembledMessage::body)
    }

    /// Body of the completed message for the given key.
    #[must_use]
    pub fn completed_body_for_key(&self, key: MessageKey) -> Option<&[u8]> {
        self.completed_messages
            .iter()
            .rev()
            .find(|m| m.message_key() == key)
            .map(AssembledMessage::body)
    }

    /// Last error, if any.
    #[must_use]
    pub fn last_error(&self) -> Option<&MessageAssemblyError> {
        match &self.last_result {
            Some(Err(e)) => Some(e),
            _ => None,
        }
    }

    /// Whether the last error is a sequence mismatch.
    #[must_use]
    pub fn is_sequence_mismatch(&self, expected: FrameSequence, found: FrameSequence) -> bool {
        matches!(
            self.last_error(),
            Some(MessageAssemblyError::Series(MessageSeriesError::SequenceMismatch {
                expected: e,
                found: f,
            })) if *e == expected && *f == found
        )
    }

    /// Whether the last error is a duplicate frame.
    #[must_use]
    pub fn is_duplicate_frame(&self, key: MessageKey, sequence: FrameSequence) -> bool {
        matches!(
            self.last_error(),
            Some(MessageAssemblyError::Series(MessageSeriesError::DuplicateFrame {
                key: k,
                sequence: s,
            })) if *k == key && *s == sequence
        )
    }

    /// Whether the last error is a missing first frame.
    #[must_use]
    pub fn is_missing_first_frame(&self, key: MessageKey) -> bool {
        matches!(
            self.last_error(),
            Some(MessageAssemblyError::Series(MessageSeriesError::MissingFirstFrame {
                key: k,
            })) if *k == key
        )
    }

    /// Whether the last error is a duplicate first frame.
    #[must_use]
    pub fn is_duplicate_first_frame(&self, key: MessageKey) -> bool {
        matches!(
            self.last_error(),
            Some(MessageAssemblyError::DuplicateFirstFrame { key: k }) if *k == key
        )
    }

    /// Whether the last error is message too large.
    #[must_use]
    pub fn is_message_too_large(&self, key: MessageKey) -> bool {
        matches!(
            self.last_error(),
            Some(MessageAssemblyError::MessageTooLarge { key: k, .. }) if *k == key
        )
    }

    /// Whether the given key was evicted.
    #[must_use]
    pub fn was_evicted(&self, key: MessageKey) -> bool { self.evicted_keys.contains(&key) }
}
