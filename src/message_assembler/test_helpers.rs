//! Shared test helper functions for message assembler tests.
//!
//! These factory functions reduce test setup duplication by providing
//! sensible defaults for `FirstFrameHeader` and `ContinuationFrameHeader`
//! construction.

use super::{ContinuationFrameHeader, FirstFrameHeader, FrameSequence, MessageKey};

/// Creates a `FirstFrameHeader` with sensible defaults.
///
/// Uses `metadata_len: 0`, `total_body_len: None`, and the provided
/// `body_len` and `is_last` values.
pub fn first_header(key: u64, body_len: usize, is_last: bool) -> FirstFrameHeader {
    FirstFrameHeader {
        message_key: MessageKey(key),
        metadata_len: 0,
        body_len,
        total_body_len: None,
        is_last,
    }
}

/// Creates a `FirstFrameHeader` with a declared total body length for early validation.
///
/// This is useful for testing early rejection based on declared message size.
pub fn first_header_with_total(key: u64, body_len: usize, total: usize) -> FirstFrameHeader {
    FirstFrameHeader {
        message_key: MessageKey(key),
        metadata_len: 0,
        body_len,
        total_body_len: Some(total),
        is_last: false,
    }
}

/// Creates a `ContinuationFrameHeader` with a sequence number.
pub fn continuation_header(
    key: u64,
    seq: u32,
    body_len: usize,
    is_last: bool,
) -> ContinuationFrameHeader {
    ContinuationFrameHeader {
        message_key: MessageKey(key),
        sequence: Some(FrameSequence(seq)),
        body_len,
        is_last,
    }
}

/// Creates a `ContinuationFrameHeader` without sequence tracking.
///
/// Used for protocols that do not track continuation frame ordering.
pub fn continuation_header_untracked(
    key: u64,
    body_len: usize,
    is_last: bool,
) -> ContinuationFrameHeader {
    ContinuationFrameHeader {
        message_key: MessageKey(key),
        sequence: None,
        body_len,
        is_last,
    }
}
