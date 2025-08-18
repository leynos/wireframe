//! Utilities for working with panic payloads.
//!
//! These helpers aid in extracting useful information from `panic!` payloads
//! for logging and diagnostics.

use std::{any::Any, fmt};

/// Wrapper that formats a panic payload when logged or displayed.
///
/// The payload is downcast to `String` or `&'static str` if possible and falls
/// back to `Debug` formatting otherwise.
///
/// ```
/// use wireframe::panic::format_panic;
/// assert_eq!(format_panic(Box::new("boom")).to_string(), "boom");
/// assert_eq!(
///     format_panic(Box::new(String::from("boom"))).to_string(),
///     "boom"
/// );
/// assert!(format_panic(Box::new(5_u32)).to_string().contains("Any"));
/// ```
#[derive(Debug)]
#[must_use]
pub struct PanicMessage(Box<dyn Any + Send>);

impl fmt::Display for PanicMessage {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if let Some(s) = self.0.downcast_ref::<String>() {
            f.write_str(s)
        } else if let Some(s) = self.0.downcast_ref::<&'static str>() {
            f.write_str(s)
        } else {
            write!(f, "{:?}", self.0)
        }
    }
}

/// Create a [`PanicMessage`] for the given payload.
pub fn format_panic(panic: Box<dyn Any + Send>) -> PanicMessage { PanicMessage(panic) }
