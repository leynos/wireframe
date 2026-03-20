//! `MessageAssemblyWorld` fixture for rstest-bdd tests.
//!
//! Provides state and helpers for message assembly multiplexing scenarios.

#[path = "message_assembly_params.rs"]
mod message_assembly_params;

use std::{
    collections::VecDeque,
    fmt,
    num::NonZeroUsize,
    time::{Duration, Instant},
};

pub use message_assembly_params::{ContinuationFrameParams, FirstFrameParams};
use rstest::fixture;
use wireframe::message_assembler::{
    AssembledMessage,
    ContinuationFrameHeader,
    EnvelopeRouting,
    FirstFrameHeader,
    FirstFrameInput,
    MessageAssemblyError,
    MessageAssemblyState,
    MessageKey,
};
/// Re-export `TestResult` from `wireframe_testing` for use in steps.
pub use wireframe_testing::TestResult;
use wireframe_testing::reassembly::{
    MessageAssemblyErrorExpectation,
    MessageAssemblySnapshot,
    assert_message_assembly_buffered_count,
    assert_message_assembly_completed,
    assert_message_assembly_completed_for_key,
    assert_message_assembly_error,
    assert_message_assembly_evicted,
    assert_message_assembly_incomplete,
};

/// Configuration for message assembly state initialisation.
#[derive(Debug, Clone, Copy)]
pub struct AssemblyConfig {
    pub max_message_size: usize,
    pub timeout_seconds: u64,
}

impl AssemblyConfig {
    pub fn new(max_message_size: usize, timeout_seconds: u64) -> Self {
        Self {
            max_message_size,
            timeout_seconds,
        }
    }
}

/// Test world for message assembly multiplexing scenarios.
#[derive(Default)]
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

// rustfmt collapses simple fixtures into one line, which triggers unused_braces.
#[rustfmt::skip]
#[fixture]
pub fn message_assembly_world() -> MessageAssemblyWorld {
    MessageAssemblyWorld::default()
}

impl MessageAssemblyWorld {
    fn snapshot(&self) -> MessageAssemblySnapshot<'_> {
        MessageAssemblySnapshot::new(
            self.last_result.as_ref(),
            &self.completed_messages,
            &self.evicted_keys,
            self.state
                .as_ref()
                .map_or(0, MessageAssemblyState::buffered_count),
            self.state
                .as_ref()
                .map_or(0, MessageAssemblyState::total_buffered_bytes),
        )
    }

    /// Initialise the assembly state with size limit and timeout.
    ///
    /// # Panics
    ///
    /// Panics if `max_message_size` is zero because `MessageAssemblyState`
    /// requires a positive size limit.
    pub fn create_state(&mut self, config: AssemblyConfig) {
        let Some(size) = NonZeroUsize::new(config.max_message_size) else {
            panic!("max_message_size must be non-zero for MessageAssemblyState");
        };
        self.state = Some(MessageAssemblyState::new(
            size,
            Duration::from_secs(config.timeout_seconds),
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
        let input = FirstFrameInput::new(
            &pending.header,
            EnvelopeRouting::default(),
            pending.metadata,
            &pending.body,
        )
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
            let input = FirstFrameInput::new(
                &pending.header,
                EnvelopeRouting::default(),
                pending.metadata,
                &pending.body,
            )
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
    ///
    /// # Errors
    ///
    /// Returns an error if time not set.
    pub fn advance_time(&mut self, secs: u64) -> TestResult {
        let Some(current) = self.current_time else {
            return Err("time not set".into());
        };
        self.current_time = Some(current + Duration::from_secs(secs));
        Ok(())
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

    /// Assert that the last message-assembly result remained incomplete.
    ///
    /// # Errors
    ///
    /// Returns an error if the last result is missing, completed, or failed.
    pub fn assert_result_incomplete(&self) -> TestResult {
        assert_message_assembly_incomplete(self.snapshot())
    }

    /// Assert that the last message-assembly operation completed with the given body.
    ///
    /// # Errors
    ///
    /// Returns an error if the last result did not complete or if the body differs.
    pub fn assert_completed_body(&self, expected: &[u8]) -> TestResult {
        assert_message_assembly_completed(self.snapshot(), expected)
    }

    /// Assert that the most recent completed message for `key` matches the expected body.
    ///
    /// # Errors
    ///
    /// Returns an error if no completed message exists for `key` or the body differs.
    pub fn assert_completed_body_for_key(&self, key: MessageKey, expected: &[u8]) -> TestResult {
        assert_message_assembly_completed_for_key(self.snapshot(), key, expected)
    }

    /// Assert that `expected` assemblies are still buffered.
    ///
    /// # Errors
    ///
    /// Returns an error if the buffered count differs.
    pub fn assert_buffered_count(&self, expected: usize) -> TestResult {
        assert_message_assembly_buffered_count(self.snapshot(), expected)
    }

    /// Assert that the last message-assembly error matches `expected`.
    ///
    /// # Errors
    ///
    /// Returns an error if no error was captured or the error does not match.
    pub fn assert_error(&self, expected: MessageAssemblyErrorExpectation) -> TestResult {
        assert_message_assembly_error(self.snapshot(), expected)
    }

    /// Assert that `key` was evicted during the most recent expiry purge.
    ///
    /// # Errors
    ///
    /// Returns an error if `key` is not in the recorded eviction list.
    pub fn assert_evicted(&self, key: MessageKey) -> TestResult {
        assert_message_assembly_evicted(self.snapshot(), key)
    }
}
