//! Declared-total validation for message assembly.
//!
//! A protocol may declare the total body length of a logical message on its
//! first frame. That declaration is a commitment, not merely a hint: it is used
//! to reject messages that cannot fit the configured size limit, and it is
//! enforced against the bytes that are actually accumulated. Treating it as
//! advisory would let a peer understate the size of the message that is
//! dispatched (`CWE-20`, `CWE-130`).
//!
//! The helpers here are pure so that the state machine in
//! [`super::state`] can apply them at each frame boundary without growing that
//! module past its size budget.
//!
//! Contract, in full:
//!
//! - a declaration smaller than the first frame's own body can never be satisfied, so the first
//!   frame is rejected;
//! - when the first frame is also the last, its body must equal the declaration exactly;
//! - each continuation must keep the accumulated body within the declaration; overshooting is
//!   rejected as soon as it is detected; and
//! - the final frame must bring the accumulated body to exactly the declared total.
//!
//! Protocols that supply `None` are unaffected.

use super::{FirstFrameHeader, MessageKey, error::MessageAssemblyError};

/// Validate the declaration carried by a first frame.
///
/// Checks that the declaration is satisfiable by the first frame alone, and
/// that a single-frame message matches its declaration exactly.
///
/// # Errors
///
/// Returns [`MessageAssemblyError::DeclaredTotalTooSmall`] when the declaration
/// is smaller than the first frame's body, or
/// [`MessageAssemblyError::DeclaredTotalMismatch`] when the first frame is also
/// the final frame and its body does not equal the declaration.
pub(super) fn validate_first_frame(
    header: &FirstFrameHeader,
    body_len: usize,
) -> Result<(), MessageAssemblyError> {
    let Some(declared) = header.total_body_len else {
        return Ok(());
    };

    let key = header.message_key;
    if declared < body_len {
        return Err(MessageAssemblyError::DeclaredTotalTooSmall {
            key,
            declared,
            body_len,
        });
    }

    if header.is_last && declared != body_len {
        return Err(MessageAssemblyError::DeclaredTotalMismatch {
            key,
            declared,
            actual: body_len,
        });
    }

    Ok(())
}

/// Validate that appending `body_len` bytes keeps the assembly within its
/// declared total.
///
/// Called before the bytes are buffered, so a peer cannot use a single
/// oversized continuation to overrun the declaration and only be detected at
/// completion.
///
/// # Errors
///
/// Returns [`MessageAssemblyError::DeclaredTotalExceeded`] when the accumulated
/// body would exceed the declared total, including when the addition would
/// overflow `usize`.
pub(super) fn validate_continuation(
    key: MessageKey,
    declared: usize,
    accumulated: usize,
    body_len: usize,
) -> Result<(), MessageAssemblyError> {
    let Some(attempted) = accumulated.checked_add(body_len) else {
        return Err(MessageAssemblyError::DeclaredTotalExceeded {
            key,
            declared,
            attempted: usize::MAX,
        });
    };

    if attempted > declared {
        return Err(MessageAssemblyError::DeclaredTotalExceeded {
            key,
            declared,
            attempted,
        });
    }

    Ok(())
}

/// Validate that a completed assembly matches its declared total exactly.
///
/// # Errors
///
/// Returns [`MessageAssemblyError::DeclaredTotalMismatch`] when the assembled
/// body length differs from the declaration. This catches an under-declared
/// multi-frame message, whose shorter-than-declared body would otherwise be
/// dispatched unchanged.
pub(super) fn validate_completion(
    key: MessageKey,
    declared: usize,
    actual: usize,
) -> Result<(), MessageAssemblyError> {
    if actual != declared {
        return Err(MessageAssemblyError::DeclaredTotalMismatch {
            key,
            declared,
            actual,
        });
    }

    Ok(())
}
