//! Assertion helpers for protocol message assembly outcomes.

use super::{
    assert_helpers::{assert_body_eq, assert_usize_field},
    message_error::{MessageAssemblyErrorExpectation, matches_message_error},
};
use crate::{
    message_assembler::{AssembledMessage, MessageAssemblyError, MessageKey},
    testkit::TestResult,
};

/// Snapshot of the observable state around a message-assembly assertion.
#[derive(Clone, Copy, Debug)]
pub struct MessageAssemblySnapshot<'a> {
    last_result: Option<&'a Result<Option<AssembledMessage>, MessageAssemblyError>>,
    completed_messages: &'a [AssembledMessage],
    evicted_keys: &'a [MessageKey],
    buffered_count: usize,
    total_buffered_bytes: usize,
}

impl<'a> MessageAssemblySnapshot<'a> {
    /// Create a snapshot from the caller's current world state.
    #[expect(
        clippy::too_many_arguments,
        reason = "snapshot constructor mirrors the observable message-assembly state"
    )]
    #[must_use]
    pub fn new(
        last_result: Option<&'a Result<Option<AssembledMessage>, MessageAssemblyError>>,
        completed_messages: &'a [AssembledMessage],
        evicted_keys: &'a [MessageKey],
        buffered_count: usize,
        total_buffered_bytes: usize,
    ) -> Self {
        Self {
            last_result,
            completed_messages,
            evicted_keys,
            buffered_count,
            total_buffered_bytes,
        }
    }

    /// Extract the last completed message from the snapshot.
    #[must_use]
    pub fn last_completed(&self) -> Option<&'a AssembledMessage> {
        match self.last_result {
            Some(Ok(Some(message))) => Some(message),
            _ => None,
        }
    }

    /// Extract the most recent completed message for a given key.
    #[must_use]
    pub fn completed_for_key(&self, key: MessageKey) -> Option<&'a AssembledMessage> {
        self.completed_messages
            .iter()
            .rev()
            .find(|message| message.message_key() == key)
    }
}

/// Assert that the last message-assembly result remained incomplete.
///
/// # Errors
///
/// Returns an error if the last result is missing, completed, or failed.
pub fn assert_message_assembly_incomplete(snapshot: MessageAssemblySnapshot<'_>) -> TestResult {
    match snapshot.last_result {
        Some(Ok(None)) => Ok(()),
        _ => Err(format!(
            "expected incomplete message assembly, got {}",
            describe_last_result(snapshot.last_result)
        )
        .into()),
    }
}

/// Assert that the last message-assembly operation completed with `expected`.
///
/// # Errors
///
/// Returns an error if the last result did not complete or if the body differs.
pub fn assert_message_assembly_completed(
    snapshot: MessageAssemblySnapshot<'_>,
    expected: &[u8],
) -> TestResult {
    let Some(message) = snapshot.last_completed() else {
        return Err(format!(
            "expected completed message assembly, got {}",
            describe_last_result(snapshot.last_result)
        )
        .into());
    };
    assert_body_eq(message.body(), expected, "completed message body")
}

/// Assert that the most recent completed message for `key` matches `expected`.
///
/// # Errors
///
/// Returns an error if no completed message exists for `key` or the body differs.
pub fn assert_message_assembly_completed_for_key(
    snapshot: MessageAssemblySnapshot<'_>,
    key: MessageKey,
    expected: &[u8],
) -> TestResult {
    let Some(message) = snapshot.completed_for_key(key) else {
        return Err(
            format!("expected completed message for key {key}, but none was recorded").into(),
        );
    };
    assert_body_eq(
        message.body(),
        expected,
        &format!("completed message body for key {key}"),
    )
}

/// Assert that the last message-assembly error matches `expected`.
///
/// # Errors
///
/// Returns an error if no error was captured or the error does not match.
pub fn assert_message_assembly_error(
    snapshot: MessageAssemblySnapshot<'_>,
    expected: MessageAssemblyErrorExpectation,
) -> TestResult {
    let Some(Err(err)) = snapshot.last_result else {
        return Err(format!(
            "expected message assembly error, got {}",
            describe_last_result(snapshot.last_result)
        )
        .into());
    };

    if matches_message_error(err, expected) {
        Ok(())
    } else {
        Err(format!("expected {expected:?}, got {err:?}").into())
    }
}

/// Assert that `expected` assemblies are still buffered.
///
/// # Errors
///
/// Returns an error if the buffered count differs.
pub fn assert_message_assembly_buffered_count(
    snapshot: MessageAssemblySnapshot<'_>,
    expected: usize,
) -> TestResult {
    assert_usize_field(snapshot.buffered_count, expected, "buffered_count")
}

/// Assert that the buffered-byte accounting equals `expected`.
///
/// # Errors
///
/// Returns an error if the buffered-byte count differs.
pub fn assert_message_assembly_total_buffered_bytes(
    snapshot: MessageAssemblySnapshot<'_>,
    expected: usize,
) -> TestResult {
    assert_usize_field(
        snapshot.total_buffered_bytes,
        expected,
        "total_buffered_bytes",
    )
}

/// Assert that `key` was evicted during the most recent expiry purge.
///
/// # Errors
///
/// Returns an error if `key` is not in the recorded eviction list.
pub fn assert_message_assembly_evicted(
    snapshot: MessageAssemblySnapshot<'_>,
    key: MessageKey,
) -> TestResult {
    if snapshot.evicted_keys.contains(&key) {
        Ok(())
    } else {
        Err(format!("expected key {key} to be evicted").into())
    }
}

fn describe_last_result(
    last_result: Option<&Result<Option<AssembledMessage>, MessageAssemblyError>>,
) -> String {
    match last_result {
        None => "no result recorded".to_string(),
        Some(Ok(None)) => "incomplete assembly".to_string(),
        Some(Ok(Some(message))) => {
            format!("completed assembly for key {}", message.message_key())
        }
        Some(Err(err)) => format!("error {err:?}"),
    }
}
