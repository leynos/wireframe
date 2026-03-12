//! Assertion helpers for protocol message assembly outcomes.

use wireframe::message_assembler::{AssembledMessage, MessageAssemblyError, MessageKey};

use super::message_error::{MessageAssemblyErrorExpectation, matches_message_error};
use crate::integration_helpers::TestResult;

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
    ///
    /// # Examples
    ///
    /// ```rust,no_run
    /// use wireframe::message_assembler::MessageKey;
    /// use wireframe_testing::reassembly::MessageAssemblySnapshot;
    ///
    /// # fn build<'a>(
    /// #     last_result: Option<&'a Result<
    /// #         Option<wireframe::message_assembler::AssembledMessage>,
    /// #         wireframe::message_assembler::MessageAssemblyError,
    /// #     >>,
    /// #     completed: &'a [wireframe::message_assembler::AssembledMessage],
    /// #     evicted: &'a [MessageKey],
    /// # ) -> MessageAssemblySnapshot<'a> {
    /// let snapshot = MessageAssemblySnapshot::new(last_result, completed, evicted, 1, 12);
    /// # snapshot
    /// # }
    /// ```
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
}

/// Assert that the last message-assembly result remained incomplete.
///
/// # Errors
///
/// Returns an error if the last result is missing, completed, or failed.
///
/// # Examples
///
/// ```rust,no_run
/// use wireframe_testing::reassembly::{
///     MessageAssemblySnapshot,
///     assert_message_assembly_incomplete,
/// };
///
/// # fn run(snapshot: MessageAssemblySnapshot<'_>) -> wireframe_testing::TestResult {
/// assert_message_assembly_incomplete(snapshot)?;
/// # Ok(())
/// # }
/// ```
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
///
/// # Examples
///
/// ```rust,no_run
/// use wireframe_testing::reassembly::{
///     MessageAssemblySnapshot,
///     assert_message_assembly_completed,
/// };
///
/// # fn run(snapshot: MessageAssemblySnapshot<'_>) -> wireframe_testing::TestResult {
/// assert_message_assembly_completed(snapshot, b"hello world")?;
/// # Ok(())
/// # }
/// ```
pub fn assert_message_assembly_completed(
    snapshot: MessageAssemblySnapshot<'_>,
    expected: &[u8],
) -> TestResult {
    let message = match snapshot.last_result {
        Some(Ok(Some(message))) => message,
        _ => {
            return Err(format!(
                "expected completed message assembly, got {}",
                describe_last_result(snapshot.last_result)
            )
            .into());
        }
    };
    assert_body_eq(message.body(), expected, "completed message body")
}

/// Assert that the most recent completed message for `key` matches `expected`.
///
/// # Errors
///
/// Returns an error if no completed message exists for `key` or the body differs.
///
/// # Examples
///
/// ```rust,no_run
/// use wireframe::message_assembler::MessageKey;
/// use wireframe_testing::reassembly::{
///     MessageAssemblySnapshot,
///     assert_message_assembly_completed_for_key,
/// };
///
/// # fn run(snapshot: MessageAssemblySnapshot<'_>) -> wireframe_testing::TestResult {
/// assert_message_assembly_completed_for_key(snapshot, MessageKey(7), b"done")?;
/// # Ok(())
/// # }
/// ```
pub fn assert_message_assembly_completed_for_key(
    snapshot: MessageAssemblySnapshot<'_>,
    key: MessageKey,
    expected: &[u8],
) -> TestResult {
    let Some(message) = snapshot
        .completed_messages
        .iter()
        .rev()
        .find(|message| message.message_key() == key)
    else {
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
///
/// # Examples
///
/// ```rust,no_run
/// use wireframe::message_assembler::{FrameSequence, MessageKey};
/// use wireframe_testing::reassembly::{
///     MessageAssemblyErrorExpectation,
///     MessageAssemblySnapshot,
///     assert_message_assembly_error,
/// };
///
/// # fn run(snapshot: MessageAssemblySnapshot<'_>) -> wireframe_testing::TestResult {
/// assert_message_assembly_error(
///     snapshot,
///     MessageAssemblyErrorExpectation::SequenceMismatch {
///         expected: FrameSequence(2),
///         found: FrameSequence(3),
///     },
/// )?;
/// # Ok(())
/// # }
/// ```
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
///
/// # Examples
///
/// ```rust,no_run
/// use wireframe_testing::reassembly::{
///     MessageAssemblySnapshot,
///     assert_message_assembly_buffered_count,
/// };
///
/// # fn run(snapshot: MessageAssemblySnapshot<'_>) -> wireframe_testing::TestResult {
/// assert_message_assembly_buffered_count(snapshot, 1)?;
/// # Ok(())
/// # }
/// ```
pub fn assert_message_assembly_buffered_count(
    snapshot: MessageAssemblySnapshot<'_>,
    expected: usize,
) -> TestResult {
    if snapshot.buffered_count == expected {
        Ok(())
    } else {
        Err(format!(
            "expected buffered_count={expected}, got {}",
            snapshot.buffered_count
        )
        .into())
    }
}

/// Assert that the buffered-byte accounting equals `expected`.
///
/// # Errors
///
/// Returns an error if the buffered-byte count differs.
///
/// # Examples
///
/// ```rust,no_run
/// use wireframe_testing::reassembly::{
///     MessageAssemblySnapshot,
///     assert_message_assembly_total_buffered_bytes,
/// };
///
/// # fn run(snapshot: MessageAssemblySnapshot<'_>) -> wireframe_testing::TestResult {
/// assert_message_assembly_total_buffered_bytes(snapshot, 10)?;
/// # Ok(())
/// # }
/// ```
pub fn assert_message_assembly_total_buffered_bytes(
    snapshot: MessageAssemblySnapshot<'_>,
    expected: usize,
) -> TestResult {
    if snapshot.total_buffered_bytes == expected {
        Ok(())
    } else {
        Err(format!(
            "expected total_buffered_bytes={expected}, got {}",
            snapshot.total_buffered_bytes
        )
        .into())
    }
}

/// Assert that `key` was evicted during the most recent expiry purge.
///
/// # Errors
///
/// Returns an error if `key` is not in the recorded eviction list.
///
/// # Examples
///
/// ```rust,no_run
/// use wireframe::message_assembler::MessageKey;
/// use wireframe_testing::reassembly::{MessageAssemblySnapshot, assert_message_assembly_evicted};
///
/// # fn run(snapshot: MessageAssemblySnapshot<'_>) -> wireframe_testing::TestResult {
/// assert_message_assembly_evicted(snapshot, MessageKey(5))?;
/// # Ok(())
/// # }
/// ```
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

fn assert_body_eq(actual: &[u8], expected: &[u8], context: &str) -> TestResult {
    if actual == expected {
        Ok(())
    } else {
        Err(format!("{context} mismatch: expected {expected:?}, got {actual:?}").into())
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
