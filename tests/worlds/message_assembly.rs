//! Test world for message assembly multiplexing and continuity validation.
#![cfg(not(loom))]

use std::{
    fmt,
    num::NonZeroUsize,
    time::{Duration, Instant},
};

use cucumber::World;
use wireframe::message_assembler::{
    AssembledMessage,
    ContinuationFrameHeader,
    FirstFrameHeader,
    FrameSequence,
    MessageAssemblyError,
    MessageAssemblyState,
    MessageKey,
    MessageSeriesError,
};

use super::TestResult;

/// Cucumber world for message assembly tests.
#[derive(Default, World)]
#[world(init = Self::new)]
pub struct MessageAssemblyWorld {
    state: Option<MessageAssemblyState>,
    current_time: Option<Instant>,
    pending_first_frames: Vec<PendingFirstFrame>,
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
            pending_first_frames: Vec::new(),
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

    /// Queue a first frame for later acceptance.
    #[expect(clippy::too_many_arguments, reason = "test helper clarity")]
    pub fn add_first_frame(&mut self, key: u64, metadata: Vec<u8>, body: Vec<u8>, is_last: bool) {
        self.pending_first_frames.push(PendingFirstFrame {
            header: FirstFrameHeader {
                message_key: MessageKey(key),
                metadata_len: metadata.len(),
                body_len: body.len(),
                total_body_len: None,
                is_last,
            },
            metadata,
            body,
        });
    }

    /// Accept the most recently queued first frame.
    ///
    /// # Errors
    ///
    /// Returns an error if no pending frames, state not initialised, or time not set.
    pub fn accept_first_frame(&mut self) -> TestResult {
        let Some(pending) = self.pending_first_frames.pop() else {
            return Err("no pending first frame".into());
        };
        let Some(state) = self.state.as_mut() else {
            return Err("state not initialised".into());
        };
        let Some(now) = self.current_time else {
            return Err("time not set".into());
        };
        self.last_result = Some(state.accept_first_frame_at(
            &pending.header,
            pending.metadata,
            &pending.body,
            now,
        ));
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

        while let Some(pending) = self.pending_first_frames.pop() {
            let result =
                state.accept_first_frame_at(&pending.header, pending.metadata, &pending.body, now);
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
    #[expect(clippy::too_many_arguments, reason = "test helper clarity")]
    pub fn accept_continuation(
        &mut self,
        key: u64,
        sequence: Option<u32>,
        body: &[u8],
        is_last: bool,
    ) -> TestResult {
        let Some(state) = self.state.as_mut() else {
            return Err("state not initialised".into());
        };
        let Some(now) = self.current_time else {
            return Err("time not set".into());
        };

        let header = ContinuationFrameHeader {
            message_key: MessageKey(key),
            sequence: sequence.map(FrameSequence),
            body_len: body.len(),
            is_last,
        };
        self.last_result = Some(state.accept_continuation_frame_at(&header, body, now));
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
    pub fn completed_body_for_key(&self, key: u64) -> Option<&[u8]> {
        self.completed_messages
            .iter()
            .rev()
            .find(|m| m.message_key() == MessageKey(key))
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
    pub fn is_sequence_mismatch(&self, expected: u32, found: u32) -> bool {
        matches!(
            self.last_error(),
            Some(MessageAssemblyError::Series(MessageSeriesError::SequenceMismatch {
                expected: e,
                found: f,
            })) if e.0 == expected && f.0 == found
        )
    }

    /// Whether the last error is a duplicate frame.
    #[must_use]
    pub fn is_duplicate_frame(&self, key: u64, sequence: u32) -> bool {
        matches!(
            self.last_error(),
            Some(MessageAssemblyError::Series(MessageSeriesError::DuplicateFrame {
                key: k,
                sequence: s,
            })) if k.0 == key && s.0 == sequence
        )
    }

    /// Whether the last error is a missing first frame.
    #[must_use]
    pub fn is_missing_first_frame(&self, key: u64) -> bool {
        matches!(
            self.last_error(),
            Some(MessageAssemblyError::Series(MessageSeriesError::MissingFirstFrame {
                key: k,
            })) if k.0 == key
        )
    }

    /// Whether the last error is a duplicate first frame.
    #[must_use]
    pub fn is_duplicate_first_frame(&self, key: u64) -> bool {
        matches!(
            self.last_error(),
            Some(MessageAssemblyError::DuplicateFirstFrame { key: k }) if k.0 == key
        )
    }

    /// Whether the last error is message too large.
    #[must_use]
    pub fn is_message_too_large(&self, key: u64) -> bool {
        matches!(
            self.last_error(),
            Some(MessageAssemblyError::MessageTooLarge { key: k, .. }) if k.0 == key
        )
    }

    /// Whether the given key was evicted.
    #[must_use]
    pub fn was_evicted(&self, key: u64) -> bool { self.evicted_keys.contains(&MessageKey(key)) }
}
