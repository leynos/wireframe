//! Shared low-level assertion primitives for reassembly modules.
//!
//! These helpers centralise common equality checks and error formatting
//! so `message` and `fragment` assertion modules stay thin and consistent.

use crate::integration_helpers::TestResult;

/// Assert that two `usize` values are equal, producing a diagnostic
/// that names the field.
pub(super) fn assert_usize_field(actual: usize, expected: usize, field_name: &str) -> TestResult {
    if actual == expected {
        Ok(())
    } else {
        Err(format!("expected {field_name}={expected}, got {actual}").into())
    }
}

/// Assert that two byte slices are equal, producing a diagnostic that
/// names the context.
pub(super) fn assert_body_eq(actual: &[u8], expected: &[u8], context: &str) -> TestResult {
    if actual == expected {
        Ok(())
    } else {
        Err(format!("{context} mismatch: expected {expected:?}, got {actual:?}").into())
    }
}
