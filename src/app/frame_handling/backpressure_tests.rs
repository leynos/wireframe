//! Unit tests for soft-limit back-pressure policy helpers.
#![expect(
    clippy::expect_used,
    reason = "Test setup intentionally uses expect for concise failure diagnostics."
)]

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
    let per_message = NonZeroUsize::new(1024).expect("1024 is non-zero");
    let per_connection = NonZeroUsize::new(100).expect("100 is non-zero");
    let in_flight = NonZeroUsize::new(100).expect("100 is non-zero");
    MemoryBudgets::new(
        BudgetBytes::new(per_message),
        BudgetBytes::new(per_connection),
        BudgetBytes::new(in_flight),
    )
}

fn state_with_buffered_bytes(buffered_bytes: usize) -> MessageAssemblyState {
    let max = NonZeroUsize::new(buffered_bytes.saturating_add(64))
        .expect("buffer size plus padding should be non-zero");
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
    let input = FirstFrameInput::new(&header, EnvelopeRouting::default(), vec![], &body)
        .expect("test first frame input should be valid");
    let result = state
        .accept_first_frame(input)
        .expect("first frame should be accepted");
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
#[case(79, false)]
#[case(80, true)]
#[case(95, true)]
fn soft_limit_pause_threshold_behaviour(
    #[case] buffered_bytes: usize,
    #[case] should_pause: bool,
    budgets: MemoryBudgets,
) {
    let state = state_with_buffered_bytes(buffered_bytes);
    assert_eq!(
        should_pause_inbound_reads(Some(&state), Some(budgets)),
        should_pause
    );
}

#[rstest]
fn uses_smallest_aggregate_budget_dimension() {
    let per_message = NonZeroUsize::new(1024).expect("1024 is non-zero");
    let per_connection = NonZeroUsize::new(200).expect("200 is non-zero");
    let in_flight = NonZeroUsize::new(100).expect("100 is non-zero");
    let budgets = MemoryBudgets::new(
        BudgetBytes::new(per_message),
        BudgetBytes::new(per_connection),
        BudgetBytes::new(in_flight),
    );
    let state = state_with_buffered_bytes(80);
    assert!(should_pause_inbound_reads(Some(&state), Some(budgets)));
}
