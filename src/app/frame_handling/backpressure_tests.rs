//! Unit tests for soft-limit back-pressure policy helpers.

use std::{num::NonZeroUsize, time::Duration};

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

#[fixture]
fn budgets() -> MemoryBudgets {
    let Some(per_message) = NonZeroUsize::new(1024) else {
        panic!("1024 is non-zero");
    };
    let Some(per_connection) = NonZeroUsize::new(100) else {
        panic!("100 is non-zero");
    };
    let Some(in_flight) = NonZeroUsize::new(100) else {
        panic!("100 is non-zero");
    };
    MemoryBudgets::new(
        BudgetBytes::new(per_message),
        BudgetBytes::new(per_connection),
        BudgetBytes::new(in_flight),
    )
}

fn state_with_buffered_bytes(buffered_bytes: usize) -> MessageAssemblyState {
    let Some(max) = NonZeroUsize::new(buffered_bytes.saturating_add(64)) else {
        panic!("buffer size plus padding should be non-zero");
    };
    let mut state = MessageAssemblyState::new(max, Duration::from_secs(30));
    if buffered_bytes == 0 {
        return state;
    }

    let body = vec![0_u8; buffered_bytes];
    let header = FirstFrameHeader {
        message_key: MessageKey(1),
        metadata_len: 0,
        body_len: buffered_bytes,
        total_body_len: None,
        is_last: false,
    };
    let input = match FirstFrameInput::new(&header, EnvelopeRouting::default(), vec![], &body) {
        Ok(input) => input,
        Err(error) => panic!("test first frame input should be valid: {error}"),
    };
    let result = match state.accept_first_frame(input) {
        Ok(result) => result,
        Err(error) => panic!("first frame should be accepted: {error}"),
    };
    assert!(
        result.is_none(),
        "first frame should start an in-flight assembly"
    );
    state
}

#[rstest]
fn should_not_pause_when_budgets_are_not_configured() {
    let state = state_with_buffered_bytes(95);
    assert!(!should_pause_inbound_reads(Some(&state), None));
}

#[rstest]
fn should_not_pause_when_state_is_not_available(budgets: MemoryBudgets) {
    assert!(!should_pause_inbound_reads(None, Some(budgets)));
}

#[rstest]
fn should_not_pause_below_soft_limit(budgets: MemoryBudgets) {
    let state = state_with_buffered_bytes(79);
    assert!(!should_pause_inbound_reads(Some(&state), Some(budgets)));
}

#[rstest]
fn should_pause_at_soft_limit(budgets: MemoryBudgets) {
    let state = state_with_buffered_bytes(80);
    assert!(should_pause_inbound_reads(Some(&state), Some(budgets)));
}

#[rstest]
fn should_pause_above_soft_limit(budgets: MemoryBudgets) {
    let state = state_with_buffered_bytes(95);
    assert!(should_pause_inbound_reads(Some(&state), Some(budgets)));
}

#[rstest]
fn uses_smallest_aggregate_budget_dimension() {
    let Some(per_message) = NonZeroUsize::new(1024) else {
        panic!("1024 is non-zero");
    };
    let Some(per_connection) = NonZeroUsize::new(200) else {
        panic!("200 is non-zero");
    };
    let Some(in_flight) = NonZeroUsize::new(100) else {
        panic!("100 is non-zero");
    };
    let budgets = MemoryBudgets::new(
        BudgetBytes::new(per_message),
        BudgetBytes::new(per_connection),
        BudgetBytes::new(in_flight),
    );
    let state = state_with_buffered_bytes(80);
    assert!(should_pause_inbound_reads(Some(&state), Some(budgets)));
}
