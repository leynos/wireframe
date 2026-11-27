//! Fixture helpers for the metadata-inclusive per-message limit.
//!
//! The ticket's 10-byte limit needs a budget-aware start, a metadata-aware
//! first frame, and an assertion that pins the exact payload sequence the
//! handler observed. Keeping them here leaves the shared inbound fixture
//! focused on its default configuration.

use std::{num::NonZeroUsize, time::Duration};

use wireframe::{
    app::{BudgetBytes, MemoryBudgets},
    test_helpers::{self, FirstFrameSpec},
};

use super::{MessageAssemblyInboundWorld, TestResult};

/// Per-message limit used by the ticket's 10-byte examples.
const MESSAGE_LIMIT: usize = 10;

/// Aggregate budget that leaves the per-message limit decisive.
const AGGREGATE_LIMIT: usize = 100;

/// How long to wait for further payloads once the first one has arrived.
const EXACT_COLLECT_TIMEOUT_MS: u64 = 200;

/// Build the budget configuration the scenario runs under.
///
/// # Errors
///
/// Returns an error if a configured limit is zero.
fn scenario_budgets() -> TestResult<MemoryBudgets> {
    let per_message = NonZeroUsize::new(MESSAGE_LIMIT).ok_or("message limit must be non-zero")?;
    let aggregate = NonZeroUsize::new(AGGREGATE_LIMIT).ok_or("aggregate limit must be non-zero")?;
    Ok(MemoryBudgets::new(
        BudgetBytes::new(per_message),
        BudgetBytes::new(aggregate),
        BudgetBytes::new(aggregate),
    ))
}

impl MessageAssemblyInboundWorld {
    /// Start the app with the ticket's 10-byte per-message limit.
    ///
    /// # Errors
    ///
    /// Returns an error if the budgets or the app fail to initialize.
    pub fn start_app_with_metadata_cap(&mut self, timeout_ms: u64) -> TestResult {
        let budgets = scenario_budgets()?;
        self.start_app_with_budgets(timeout_ms, Some(budgets))
    }

    /// Send a first frame described by `spec`, metadata included.
    ///
    /// # Errors
    ///
    /// Returns an error if the frame cannot be encoded or sent.
    pub fn send_first_frame_with(&mut self, spec: FirstFrameSpec<'_>) -> TestResult {
        let payload = test_helpers::first_frame_payload_with_metadata(spec)?;
        self.send_payload(payload)
    }

    /// Assert that the handler observed exactly `expected`, in order.
    ///
    /// The handler channel is the barrier: the method waits for payloads until
    /// the collection window expires with nothing further, so a stray dispatch
    /// cannot slip past the comparison.
    ///
    /// # Errors
    ///
    /// Returns an error if the observed payload sequence differs.
    pub fn assert_received_payloads(&mut self, expected: &[&str]) -> TestResult {
        self.collect_observed_for(Duration::from_millis(EXACT_COLLECT_TIMEOUT_MS))?;
        let actual: Vec<&[u8]> = self.observed_payloads.iter().map(Vec::as_slice).collect();
        let wanted: Vec<&[u8]> = expected.iter().map(|payload| payload.as_bytes()).collect();
        if actual == wanted {
            return Ok(());
        }
        Err(format!("expected payloads {wanted:?}, got {actual:?}").into())
    }
}
