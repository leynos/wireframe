//! Utilities for working with panic payloads.
//!
//! These helpers aid in extracting useful information from `panic!` payloads
//! for logging and diagnostics.

use std::any::Any;

/// Format a panic payload into a human-readable message.
///
/// The payload is downcast to `String` or `&'static str` if possible and falls
/// back to `Debug` formatting otherwise.
///
/// ```
/// use wireframe::panic::format_panic;
/// assert_eq!(format_panic(Box::new("boom")), "boom");
/// assert_eq!(format_panic(Box::new(String::from("boom"))), "boom");
/// assert_eq!(format_panic(Box::new(5_u32)), "Any");
/// ```
#[must_use]
pub fn format_panic(panic: Box<dyn Any + Send>) -> String {
    match panic.downcast::<String>() {
        Ok(s) => *s,
        Err(panic) => match panic.downcast::<&'static str>() {
            Ok(s) => (*s).to_string(),
            Err(panic) => format!("{panic:?}"),
        },
    }
}
