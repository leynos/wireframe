//! Grammar for the ordered hook-event log the recording protocol produces.
//!
//! One response source completes exactly once, and a completion is the
//! contiguous tail `StreamEnd, [BeforeSend(terminator)], CommandEnd`. Every
//! other event is a dispatch the contract does not allow, except the frames a
//! source emits before it completes. This module names that grammar once so
//! the generated mixed-source cases and their example regressions share it.
//! Include it after `recording_protocol` with
//! `#[path = "common/hook_event_grammar.rs"] mod hook_event_grammar;`.

use std::slice::Iter;

use super::recording_protocol::HookEvent;

/// Source tag for the stream-end terminator.
pub const TERMINATOR_TAG: u8 = 0x05;

/// Iterator over the recorded events, consumed by the grammar checks.
type EventIter<'a> = Iter<'a, HookEvent<Vec<u8>>>;

/// Validate the event grammar and return every frame `before_send` observed.
///
/// The actor dispatches `StreamEnd`, then the terminator when the protocol
/// supplies one, then the deferred completion hook, so one source completion
/// is the contiguous tail `StreamEnd, [BeforeSend(terminator)], CommandEnd`.
/// Any other event is a dispatch the contract does not allow. A
/// terminator-tagged frame is accepted only in that tail: one that appears
/// anywhere else is a duplicate terminator, not an ordinary frame, and fails
/// the grammar instead of being filtered out later.
pub fn before_send_order(
    events: &[HookEvent<Vec<u8>>],
    terminator: Option<&[u8]>,
) -> Result<Vec<Vec<u8>>, String> {
    let mut iter = events.iter();
    if iter.next() != Some(&HookEvent::Setup) {
        return Err(format!("setup must run first, got {events:?}"));
    }
    let mut frames = Vec::new();
    let mut completions = 0_usize;
    while let Some(event) = iter.next() {
        match event {
            HookEvent::BeforeSend(frame) => {
                if frame.first() == Some(&TERMINATOR_TAG) {
                    return Err(format!(
                        "terminator-tagged frame outside the completion tail: {frame:?}"
                    ));
                }
                frames.push(frame.clone());
            }
            HookEvent::StreamEnd => {
                completions += 1;
                collect_completion(&mut iter, terminator, &mut frames)?;
            }
            other => return Err(format!("unexpected event {other:?}")),
        }
    }
    if completions != 1 {
        return Err(format!(
            "expected exactly one source completion, got {completions}: {events:?}"
        ));
    }
    Ok(frames)
}

/// Consume the terminator and completion hooks that follow a stream end.
fn collect_completion(
    rest: &mut EventIter<'_>,
    terminator: Option<&[u8]>,
    frames: &mut Vec<Vec<u8>>,
) -> Result<(), String> {
    if let Some(expected) = terminator {
        take_terminator(rest, expected, frames)?;
    }
    take_command_end(rest)
}

/// Consume the protocol-supplied terminator that follows a stream end.
fn take_terminator(
    rest: &mut EventIter<'_>,
    expected: &[u8],
    frames: &mut Vec<Vec<u8>>,
) -> Result<(), String> {
    match rest.next() {
        Some(HookEvent::BeforeSend(frame)) if frame.as_slice() == expected => {
            frames.push(frame.clone());
            Ok(())
        }
        other => Err(format!(
            "expected the terminator frame after stream end, got {other:?}"
        )),
    }
}

/// Consume the deferred completion hook that ends a source.
fn take_command_end(rest: &mut EventIter<'_>) -> Result<(), String> {
    match rest.next() {
        Some(HookEvent::CommandEnd) => Ok(()),
        other => Err(format!("expected command end, got {other:?}")),
    }
}
