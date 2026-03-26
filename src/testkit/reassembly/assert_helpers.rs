//! Shared low-level assertion primitives for reassembly modules.

use super::super::TestResult;

pub(super) fn assert_usize_field(actual: usize, expected: usize, field_name: &str) -> TestResult {
    if actual == expected {
        Ok(())
    } else {
        Err(format!("expected {field_name}={expected}, got {actual}").into())
    }
}

pub(super) fn assert_body_eq(actual: &[u8], expected: &[u8], context: &str) -> TestResult {
    if actual == expected {
        Ok(())
    } else {
        Err(format!("{context} mismatch: expected {expected:?}, got {actual:?}").into())
    }
}
