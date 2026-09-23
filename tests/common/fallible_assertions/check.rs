//! Fallible predicate checks shared by integration-test crates.

/// Return an error containing `message` when a predicate is false.
///
/// # Examples
///
/// ```rust,ignore
/// let successful = check(true, "the route is registered");
/// let failed = check(false, "the route is missing");
/// // `successful` is `Ok(())`; `failed` contains the supplied message.
/// ```
///
/// # Errors
///
/// Returns `Err` with the supplied message when `condition` is false.
pub(super) fn check(condition: bool, message: impl Into<String>) -> Result<(), String> {
    if condition {
        Ok(())
    } else {
        let message = message.into();
        Err(message)
    }
}
