//! Utilities for formatting panic payloads.
//!
//! This module centralises panic formatting so logs and errors can present
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
    if let Some(message) = panic.downcast_ref::<&'static str>() {
        (*message).to_owned()
    } else if let Some(message) = panic.downcast_ref::<String>() {
        message.clone()
    } else {
        format!("{panic:?}")
    }
}
