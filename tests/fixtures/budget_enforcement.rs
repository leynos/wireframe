//! Behavioural test world for budget enforcement (8.3.2).

use std::{
    num::NonZeroUsize,
    str::FromStr,
    time::{Duration, Instant},
};

use rstest::fixture;
use wireframe::message_assembler::{
    ContinuationFrameHeader,
    EnvelopeRouting,
    FirstFrameHeader,
    FirstFrameInput,
    FrameSequence,
    MessageAssemblyError,
    MessageAssemblyState,
    MessageKey,
};
pub use wireframe_testing::TestResult;

/// Configuration bundle for `init_budgeted_state`.
///
/// Parsed from the Gherkin step text as
/// `"max_message_size / timeout_secs / connection_budget / in_flight_budget"`.
#[derive(Clone, Copy)]
pub struct BudgetedStateConfig {
    pub max_message_size: usize,
    pub timeout_secs: u64,
    pub connection_budget: usize,
    pub in_flight_budget: usize,
}

impl FromStr for BudgetedStateConfig {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let mut parts = s.split('/').map(str::trim);
        let msg = parts.next().ok_or("missing max_message_size")?;
        let timeout = parts.next().ok_or("missing timeout_secs")?;
        let conn = parts.next().ok_or("missing connection_budget")?;
        let flight = parts.next().ok_or("missing in_flight_budget")?;
        Ok(Self {
            max_message_size: msg.parse().map_err(|e| format!("max_message_size: {e}"))?,
            timeout_secs: timeout.parse().map_err(|e| format!("timeout_secs: {e}"))?,
            connection_budget: conn
                .parse()
                .map_err(|e| format!("connection_budget: {e}"))?,
            in_flight_budget: flight
                .parse()
                .map_err(|e| format!("in_flight_budget: {e}"))?,
        })
    }
}

/// Continuation frame descriptor.
#[derive(Clone, Copy)]
pub struct ContinuationInput {
    pub key: u64,
    pub sequence: u32,
    pub body_len: usize,
    pub is_last: bool,
}

/// Behavioural test world for budget enforcement scenarios.
#[derive(Debug)]
pub struct BudgetEnforcementWorld {
    state: MessageAssemblyState,
    last_error: Option<MessageAssemblyError>,
    last_accepted: bool,
    last_completed: bool,
    epoch: Option<Instant>,
}

impl Default for BudgetEnforcementWorld {
    fn default() -> Self {
        // Use a forgiving default so that scenarios which forget to call
        // init_budgeted_state fail with a clear budget error rather than a
        // confusing MessageTooLarge on any multi-byte body.
        let max = NonZeroUsize::new(64 * 1024).unwrap_or(NonZeroUsize::MIN);
        Self {
            state: MessageAssemblyState::new(max, Duration::from_secs(30)),
            last_error: None,
            last_accepted: false,
            last_completed: false,
            epoch: None,
        }
    }
}

/// Fixture for `BudgetEnforcementWorld`.
#[fixture]
#[rustfmt::skip]
pub fn budget_enforcement_world() -> BudgetEnforcementWorld {
    BudgetEnforcementWorld::default()
}

impl BudgetEnforcementWorld {
    /// Re-initialise state with explicit budgets.
    pub fn init_budgeted_state(&mut self, cfg: BudgetedStateConfig) -> TestResult {
        let max_message_size =
            NonZeroUsize::new(cfg.max_message_size).ok_or("max_message_size must be non-zero")?;
        let connection =
            NonZeroUsize::new(cfg.connection_budget).ok_or("connection_budget must be non-zero")?;
        let in_flight =
            NonZeroUsize::new(cfg.in_flight_budget).ok_or("in_flight_budget must be non-zero")?;
        self.state = MessageAssemblyState::with_budgets(
            max_message_size,
            Duration::from_secs(cfg.timeout_secs),
            Some(connection),
            Some(in_flight),
        );
        Ok(())
    }

    /// Set the epoch for time-based tests.
    pub fn set_epoch(&mut self) { self.epoch = Some(Instant::now()); }

    /// Record the result of an assembly operation.
    fn record_result(
        &mut self,
        result: Result<
            Option<wireframe::message_assembler::AssembledMessage>,
            MessageAssemblyError,
        >,
    ) {
        match result {
            Ok(None) => {
                self.last_accepted = true;
                self.last_completed = false;
                self.last_error = None;
            }
            Ok(Some(_)) => {
                self.last_accepted = true;
                self.last_completed = true;
                self.last_error = None;
            }
            Err(e) => {
                self.last_accepted = false;
                self.last_completed = false;
                self.last_error = Some(e);
            }
        }
    }

    /// Build a first-frame header and body from key, body length, and finality.
    fn build_first_input(key: u64, body_len: usize, is_last: bool) -> (Vec<u8>, FirstFrameHeader) {
        (
            vec![0u8; body_len],
            FirstFrameHeader {
                message_key: MessageKey(key),
                metadata_len: 0,
                body_len,
                total_body_len: None,
                is_last,
            },
        )
    }

    /// Submit a multi-frame first frame.
    pub fn accept_first_frame(&mut self, key: u64, body_len: usize) -> TestResult {
        let (body, header) = Self::build_first_input(key, body_len, false);
        let input = FirstFrameInput::new(&header, EnvelopeRouting::default(), vec![], &body)
            .map_err(|e| e.to_string())?;
        let result = self.state.accept_first_frame(input);
        self.record_result(result);
        Ok(())
    }

    /// Submit a multi-frame first frame at the stored epoch.
    pub fn accept_first_frame_at_epoch(&mut self, key: u64, body_len: usize) -> TestResult {
        let now = self.epoch.ok_or("epoch not set")?;
        let (body, header) = Self::build_first_input(key, body_len, false);
        let input = FirstFrameInput::new(&header, EnvelopeRouting::default(), vec![], &body)
            .map_err(|e| e.to_string())?;
        let result = self.state.accept_first_frame_at(input, now);
        self.record_result(result);
        Ok(())
    }

    /// Submit a first frame that is expected to be rejected.
    pub fn reject_first_frame(&mut self, key: u64, body_len: usize) -> TestResult {
        self.accept_first_frame(key, body_len)?;
        if self.last_accepted {
            return Err("expected first frame to be rejected, but it was accepted".into());
        }
        Ok(())
    }

    /// Submit a single-frame message.
    pub fn accept_single_frame(&mut self, key: u64, body_len: usize) -> TestResult {
        let (body, header) = Self::build_first_input(key, body_len, true);
        let input = FirstFrameInput::new(&header, EnvelopeRouting::default(), vec![], &body)
            .map_err(|e| e.to_string())?;
        let result = self.state.accept_first_frame(input);
        self.record_result(result);
        Ok(())
    }

    /// Submit a continuation frame.
    pub fn accept_continuation(&mut self, input: ContinuationInput) {
        let body = vec![0u8; input.body_len];
        let header = ContinuationFrameHeader {
            message_key: MessageKey(input.key),
            sequence: Some(FrameSequence(input.sequence)),
            body_len: input.body_len,
            is_last: input.is_last,
        };
        let result = self.state.accept_continuation_frame(&header, &body);
        self.record_result(result);
    }

    /// Submit a continuation frame that is expected to be rejected.
    pub fn reject_continuation(&mut self, key: u64, sequence: u32, body_len: usize) -> TestResult {
        self.accept_continuation(ContinuationInput {
            key,
            sequence,
            body_len,
            is_last: false,
        });
        if self.last_accepted {
            return Err("expected continuation to be rejected, but it was accepted".into());
        }
        Ok(())
    }

    /// Purge expired assemblies at a given offset from epoch.
    pub fn purge_at_offset(&mut self, offset_secs: u64) -> TestResult {
        let epoch = self.epoch.ok_or("epoch not set")?;
        let future = epoch + Duration::from_secs(offset_secs);
        self.state.purge_expired_at(future);
        Ok(())
    }

    // ---- Assertions ----

    pub fn assert_accepted(&self) -> TestResult {
        if !self.last_accepted {
            let detail = self
                .last_error
                .as_ref()
                .map_or("(no error captured)".to_string(), |e| format!("{e}"));
            return Err(format!("expected frame to be accepted, got error: {detail}").into());
        }
        Ok(())
    }

    pub fn assert_completed(&self) -> TestResult {
        if !self.last_completed {
            return Err("expected message to complete".into());
        }
        Ok(())
    }

    pub fn assert_error_kind(&self, expected: &str) -> TestResult {
        let err = self
            .last_error
            .as_ref()
            .ok_or("expected an error, but none was captured")?;
        let actual = match err {
            MessageAssemblyError::ConnectionBudgetExceeded { .. } => "ConnectionBudgetExceeded",
            MessageAssemblyError::InFlightBudgetExceeded { .. } => "InFlightBudgetExceeded",
            MessageAssemblyError::MessageTooLarge { .. } => "MessageTooLarge",
            MessageAssemblyError::DuplicateFirstFrame { .. } => "DuplicateFirstFrame",
            MessageAssemblyError::Series(_) => "Series",
        };
        if actual != expected {
            return Err(format!("expected error kind '{expected}', got '{actual}'").into());
        }
        Ok(())
    }

    pub fn assert_total_buffered_bytes(&self, expected: usize) -> TestResult {
        let actual = self.state.total_buffered_bytes();
        if actual != expected {
            return Err(format!("expected total_buffered_bytes={expected}, got {actual}").into());
        }
        Ok(())
    }

    pub fn assert_buffered_count(&self, expected: usize) -> TestResult {
        let actual = self.state.buffered_count();
        if actual != expected {
            return Err(format!("expected buffered_count={expected}, got {actual}").into());
        }
        Ok(())
    }
}
