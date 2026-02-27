//! Unit tests for memory budget pressure policy helpers.

use std::{error::Error, io, num::NonZeroUsize, time::Duration};

use rstest::{fixture, rstest};

use super::{
    backpressure::{
        MemoryPressureAction,
        has_hard_cap_been_breached,
        resolve_effective_budgets,
        should_pause_inbound_reads,
    },
    evaluate_memory_pressure,
};
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

fn custom_budgets(
    per_message: usize,
    per_connection: usize,
    in_flight: usize,
) -> TestResult<MemoryBudgets> {
    let per_message = NonZeroUsize::new(per_message)
        .ok_or_else(|| io::Error::other("per_message must be > 0"))?;
    let per_connection = NonZeroUsize::new(per_connection)
        .ok_or_else(|| io::Error::other("per_connection must be > 0"))?;
    let in_flight =
        NonZeroUsize::new(in_flight).ok_or_else(|| io::Error::other("in_flight must be > 0"))?;
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
            concat!(
                "soft limit mismatch: buffered_bytes={}, expected={}, ",
                "actual={}"
            ),
            buffered_bytes, should_pause, actual
        ))
        .into());
    }
    Ok(())
}

#[rstest]
fn uses_smallest_aggregate_budget_dimension() -> TestResult {
    let budgets = custom_budgets(1024, 200, 100)?;
    let state = state_with_buffered_bytes(80)?;
    if !should_pause_inbound_reads(Some(&state), Some(budgets)) {
        return Err(io::Error::other("expected pause at derived soft limit threshold").into());
    }
    Ok(())
}

// ---------- hard-cap policy tests ----------

#[rstest]
fn hard_cap_not_breached_when_budgets_are_not_configured() -> TestResult {
    let state = state_with_buffered_bytes(200)?;
    if has_hard_cap_been_breached(Some(&state), None) {
        return Err(
            io::Error::other("did not expect breach when budgets are not configured").into(),
        );
    }
    Ok(())
}

#[rstest]
fn hard_cap_not_breached_when_state_is_not_available(
    budgets: TestResult<MemoryBudgets>,
) -> TestResult {
    if has_hard_cap_been_breached(None, Some(budgets?)) {
        return Err(io::Error::other("did not expect breach when state is not available").into());
    }
    Ok(())
}

#[rstest]
#[case(99, false)]
#[case(100, false)]
#[case(101, true)]
fn hard_cap_threshold_behaviour(
    #[case] buffered_bytes: usize,
    #[case] expected_breached: bool,
    budgets: TestResult<MemoryBudgets>,
) -> TestResult {
    let state = state_with_buffered_bytes(buffered_bytes)?;
    let actual = has_hard_cap_been_breached(Some(&state), Some(budgets?));
    if actual != expected_breached {
        return Err(io::Error::other(format!(
            concat!(
                "hard cap mismatch: buffered_bytes={}, expected={}, ",
                "actual={}"
            ),
            buffered_bytes, expected_breached, actual
        ))
        .into());
    }
    Ok(())
}

#[rstest]
fn hard_cap_uses_smallest_aggregate_budget_dimension() -> TestResult {
    let budgets = custom_budgets(1024, 200, 100)?;
    let state = state_with_buffered_bytes(101)?;
    if !has_hard_cap_been_breached(Some(&state), Some(budgets)) {
        return Err(io::Error::other("expected breach at smallest aggregate dimension").into());
    }
    Ok(())
}

// ---------- combined evaluate_memory_pressure tests ----------

/// Compare pressure actions by variant, ignoring the `Duration` payload in
/// `Pause`.
fn action_matches(actual: &MemoryPressureAction, expected: &MemoryPressureAction) -> bool {
    matches!(
        (actual, expected),
        (
            MemoryPressureAction::Continue,
            MemoryPressureAction::Continue
        ) | (
            MemoryPressureAction::Pause(_),
            MemoryPressureAction::Pause(_)
        ) | (MemoryPressureAction::Abort, MemoryPressureAction::Abort)
    )
}

/// Expected variant tag for the parameterised `evaluate_memory_pressure` test.
/// We cannot pass `MemoryPressureAction` directly in `#[case]` attributes
/// because the `Pause` variant contains a `Duration`, so we use a simple tag
/// that the test body maps to the real variant for comparison.
#[derive(Clone, Copy, Debug)]
enum ExpectedAction {
    Continue,
    Pause,
    Abort,
}

impl ExpectedAction {
    const fn to_representative(self) -> MemoryPressureAction {
        match self {
            Self::Continue => MemoryPressureAction::Continue,
            Self::Pause => MemoryPressureAction::Pause(Duration::ZERO),
            Self::Abort => MemoryPressureAction::Abort,
        }
    }
}

#[rstest]
#[case::no_budgets_configured(200, false, ExpectedAction::Continue)]
#[case::below_soft_limit(50, true, ExpectedAction::Continue)]
#[case::at_soft_limit(80, true, ExpectedAction::Pause)]
#[case::above_hard_cap_abort_takes_priority(101, true, ExpectedAction::Abort)]
fn evaluate_memory_pressure_behaviour(
    #[case] buffered_bytes: usize,
    #[case] use_budgets: bool,
    #[case] expected: ExpectedAction,
    budgets: TestResult<MemoryBudgets>,
) -> TestResult {
    let budget_config = if use_budgets { Some(budgets?) } else { None };
    let state = state_with_buffered_bytes(buffered_bytes)?;
    let actual = evaluate_memory_pressure(Some(&state), budget_config);
    let representative = expected.to_representative();
    if !action_matches(&actual, &representative) {
        return Err(io::Error::other(format!(
            "buffered_bytes={buffered_bytes}: expected {representative:?}, got {actual:?}"
        ))
        .into());
    }
    Ok(())
}

#[rstest]
fn evaluate_pause_uses_expected_duration(budgets: TestResult<MemoryBudgets>) -> TestResult {
    let state = state_with_buffered_bytes(80)?;
    let action = evaluate_memory_pressure(Some(&state), Some(budgets?));
    match action {
        MemoryPressureAction::Pause(duration) => {
            if duration != Duration::from_millis(5) {
                return Err(io::Error::other(format!(
                    "expected 5 ms pause duration, got {duration:?}"
                ))
                .into());
            }
            Ok(())
        }
        other => Err(io::Error::other(format!("expected Pause, got {other:?}")).into()),
    }
}

// ---------- resolve_effective_budgets tests ----------

#[rstest]
fn resolve_returns_explicit_budgets_when_configured() -> TestResult {
    let explicit = custom_budgets(512, 2048, 4096)?;
    let resolved = resolve_effective_budgets(Some(explicit), 1024);
    if resolved != explicit {
        return Err(io::Error::other(format!(
            "expected explicit budgets {explicit:?}, got {resolved:?}"
        ))
        .into());
    }
    Ok(())
}

#[rstest]
fn resolve_returns_derived_budgets_when_none() -> TestResult {
    let resolved = resolve_effective_budgets(None, 1024);
    if resolved.bytes_per_message().as_usize() != 16_384 {
        return Err(io::Error::other(format!(
            "expected derived bytes_per_message=16384, got {}",
            resolved.bytes_per_message().as_usize()
        ))
        .into());
    }
    if resolved.bytes_per_connection().as_usize() != 65_536 {
        return Err(io::Error::other(format!(
            "expected derived bytes_per_connection=65536, got {}",
            resolved.bytes_per_connection().as_usize()
        ))
        .into());
    }
    if resolved.bytes_in_flight().as_usize() != 65_536 {
        return Err(io::Error::other(format!(
            "expected derived bytes_in_flight=65536, got {}",
            resolved.bytes_in_flight().as_usize()
        ))
        .into());
    }
    Ok(())
}

#[rstest]
fn resolve_derived_budgets_change_with_frame_budget() -> TestResult {
    let small = resolve_effective_budgets(None, 512);
    let large = resolve_effective_budgets(None, 2048);
    if small.bytes_per_message().as_usize() >= large.bytes_per_message().as_usize() {
        return Err(io::Error::other(format!(
            "expected smaller frame budget to yield smaller per_message: {} vs {}",
            small.bytes_per_message().as_usize(),
            large.bytes_per_message().as_usize()
        ))
        .into());
    }
    if small.bytes_per_connection().as_usize() >= large.bytes_per_connection().as_usize() {
        return Err(io::Error::other(format!(
            "expected smaller frame budget to yield smaller per_connection: {} vs {}",
            small.bytes_per_connection().as_usize(),
            large.bytes_per_connection().as_usize()
        ))
        .into());
    }
    Ok(())
}
