//! Pressure coverage for the fixed per-assembly entry charge (issue #830).

use std::{io, num::NonZeroUsize, time::Duration};

use rstest::rstest;

use super::{MemoryPressureAction, TestResult, action_matches, budgets, evaluate_memory_pressure};
use crate::{
    app::MemoryBudgets,
    message_assembler::{
        EnvelopeRouting,
        FirstFrameHeader,
        FirstFrameInput,
        MessageAssemblyState,
        MessageKey,
    },
};

/// Build a state holding one assembly with no payload bytes.
fn state_with_empty_assembly() -> TestResult<MessageAssemblyState> {
    let max = NonZeroUsize::new(1024).ok_or_else(|| io::Error::other("1024 is non-zero"))?;
    let mut state = MessageAssemblyState::new(max, Duration::from_secs(30));
    let header = FirstFrameHeader {
        message_key: MessageKey(1),
        metadata_len: 0,
        body_len: 0,
        total_body_len: None,
        is_last: false,
    };
    let input = FirstFrameInput::new(&header, EnvelopeRouting::default(), vec![], &[])?;
    let result = state.accept_first_frame(input)?;
    if result.is_some() {
        return Err(io::Error::other("empty first frame should start an assembly").into());
    }
    Ok(state)
}

#[rstest]
fn empty_assembly_entry_charge_contributes_to_pressure(
    budgets: TestResult<MemoryBudgets>,
) -> TestResult {
    let state = state_with_empty_assembly()?;
    let resolved = budgets?;

    // The empty entry is charged the fixed overhead, so 16 fragment bytes reach
    // the 80% soft threshold even though no payload is buffered.
    let pause = evaluate_memory_pressure(Some(&state), Some(resolved), 16);
    if !action_matches(&pause, &MemoryPressureAction::Pause(Duration::ZERO)) {
        return Err(io::Error::other(format!(
            "expected the entry charge to reach the soft limit, got {pause:?}"
        ))
        .into());
    }

    // The same charge plus 37 fragment bytes strictly exceeds the hard cap.
    let abort = evaluate_memory_pressure(Some(&state), Some(resolved), 37);
    if !action_matches(&abort, &MemoryPressureAction::Abort) {
        return Err(io::Error::other(format!(
            "expected the entry charge to breach the hard cap, got {abort:?}"
        ))
        .into());
    }
    Ok(())
}
