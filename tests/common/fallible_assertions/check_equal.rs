//! Fallible equality checks for borrowed values in integration-test crates.

/// Compare possibly unsized values and report their context on mismatch.
///
/// # Examples
///
/// ```rust,ignore
/// let matched = check_equal(&"ready", &"ready", "state");
/// let mismatched = check_equal(&"waiting", &"ready", "state");
/// // `matched` is `Ok(())`; the error says `state: expected \"ready\", got \"waiting\"`.
/// ```
///
/// # Errors
///
/// Returns `Err` when `actual` does not equal `expected`, formatted with their
/// debug representations and `context`.
pub(super) fn check_equal<A, E>(actual: &A, expected: &E, context: &str) -> Result<(), String>
where
    A: std::fmt::Debug + PartialEq<E> + ?Sized,
    E: std::fmt::Debug + ?Sized,
{
    if actual == expected {
        Ok(())
    } else {
        Err(format!("{context}: expected {expected:?}, got {actual:?}"))
    }
}
