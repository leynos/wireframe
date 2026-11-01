//! Utilities for formatting panic payloads.
//!
//! This module centralizes panic formatting so logs and errors can present
//! consistent messages.

use std::any::Any;

/// Format a panic payload into a `String`.
///
/// String and string-slice panic payloads preserve their original message.
/// Other payload types fall back to `Debug` formatting.
///
/// # Examples
/// ```
/// # use std::any::Any;
/// # use wireframe::panic::format_panic;
/// let payload: Box<dyn Any + Send> = Box::new("boom");
/// assert_eq!(format_panic(&payload), "boom");
/// ```
#[must_use]
pub fn format_panic(panic: &Box<dyn Any + Send>) -> String {
    panic.downcast_ref::<&'static str>().map_or_else(
        || {
            panic
                .downcast_ref::<String>()
                .map_or_else(|| format!("{panic:?}"), Clone::clone)
        },
        |message| (*message).to_owned(),
    )
}
