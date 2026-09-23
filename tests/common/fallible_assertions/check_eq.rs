//! Fallible equality checks for copyable values in integration-test crates.

/// Compare two values and report their context and debug representations.
///
/// # Examples
///
/// ```rust,ignore
/// let matched = check_eq(3, 3, "item count");
/// let mismatched = check_eq(2, 3, "item count");
/// // `matched` is `Ok(())`; the error says `item count: expected 3, got 2`.
/// ```
///
/// # Errors
///
/// Returns `Err` when the values differ, formatted as
/// `"{message}: expected {expected:?}, got {actual:?}"`.
pub(super) fn check_eq<T>(actual: T, expected: T, message: &str) -> Result<(), String>
where
    T: std::fmt::Debug + PartialEq + Copy,
{
    if actual == expected {
        Ok(())
    } else {
        Err(format!("{message}: expected {expected:?}, got {actual:?}"))
    }
}
