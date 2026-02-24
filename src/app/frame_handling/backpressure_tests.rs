//! Unit tests for soft-limit back-pressure policy helpers.

use std::{error::Error, io, num::NonZeroUsize, time::Duration};

use rstest::{fixture, rstest};

use super::should_pause_inbound_reads;
use crate::{
    app::{BudgetBytes, MemoryBudgets},
    message_assembler::{
        EnvelopeRouting,
        FirstFrameHeader,
        FirstFrameInput,
        MessageAssemblyState,
        MessageKey,
    },
};

type TestResult<T = ()> = Result<T, Box<dyn Error>>;

#[fixture]
fn budgets() -> TestResult<MemoryBudgets> {
    let per_message =
        NonZeroUsize::new(1024).ok_or_else(|| io::Error::other("1024 is non-zero"))?;
    let per_connection =
        NonZeroUsize::new(100).ok_or_else(|| io::Error::other("100 is non-zero"))?;
    let in_flight = NonZeroUsize::new(100).ok_or_else(|| io::Error::other("100 is non-zero"))?;
    Ok(MemoryBudgets::new(
        BudgetBytes::new(per_message),
        BudgetBytes::new(per_connection),
        BudgetBytes::new(in_flight),
    ))
}

fn state_with_buffered_bytes(buffered_bytes: usize) -> TestResult<MessageAssemblyState> {
    let max = NonZeroUsize::new(buffered_bytes.saturating_add(64))
        .ok_or_else(|| io::Error::other("buffer size plus padding should be non-zero"))?;
    let mut state = MessageAssemblyState::new(max, Duration::from_secs(30));
    if buffered_bytes == 0 {
        return Ok(state);
    }

    let body = vec![0_u8; buffered_bytes];
    let header = FirstFrameHeader {
        message_key: MessageKey(1),
        metadata_len: 0,
        body_len: buffered_bytes,
        total_body_len: None,
        is_last: false,
    };
    let input = FirstFrameInput::new(&header, EnvelopeRouting::default(), vec![], &body)?;
    let result = state.accept_first_frame(input)?;
    if result.is_some() {
        return Err(io::Error::other("first frame should start an in-flight assembly").into());
    }
    Ok(state)
}

#[rstest]
fn should_not_pause_when_budgets_are_not_configured() -> TestResult {
    let state = state_with_buffered_bytes(95)?;
    if should_pause_inbound_reads(Some(&state), None) {
        return Err(
            io::Error::other("did not expect pause when budgets are not configured").into(),
        );
    }
    Ok(())
}

#[rstest]
fn should_not_pause_when_state_is_not_available(budgets: TestResult<MemoryBudgets>) -> TestResult {
    if should_pause_inbound_reads(None, Some(budgets?)) {
        return Err(io::Error::other("did not expect pause when state is not available").into());
    }
    Ok(())
}

#[rstest]
#[case(79, false)]
#[case(80, true)]
#[case(95, true)]
fn soft_limit_pause_threshold_behaviour(
    #[case] buffered_bytes: usize,
    #[case] should_pause: bool,
    budgets: TestResult<MemoryBudgets>,
) -> TestResult {
    let state = state_with_buffered_bytes(buffered_bytes)?;
    let actual = should_pause_inbound_reads(Some(&state), Some(budgets?));
    if actual != should_pause {
        return Err(io::Error::other(format!(
            "soft limit mismatch: buffered_bytes={buffered_bytes}, expected={should_pause}, \
             actual={actual}"
        ))
        .into());
    }
    Ok(())
}

#[rstest]
fn uses_smallest_aggregate_budget_dimension() -> TestResult {
    let per_message =
        NonZeroUsize::new(1024).ok_or_else(|| io::Error::other("1024 is non-zero"))?;
    let per_connection =
        NonZeroUsize::new(200).ok_or_else(|| io::Error::other("200 is non-zero"))?;
    let in_flight = NonZeroUsize::new(100).ok_or_else(|| io::Error::other("100 is non-zero"))?;
    let budgets = MemoryBudgets::new(
        BudgetBytes::new(per_message),
        BudgetBytes::new(per_connection),
        BudgetBytes::new(in_flight),
    );
    let state = state_with_buffered_bytes(80)?;
    if !should_pause_inbound_reads(Some(&state), Some(budgets)) {
        return Err(io::Error::other("expected pause at derived soft limit threshold").into());
    }
    Ok(())
}
