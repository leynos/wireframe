//! Utilities for formatting panic payloads.
//!
//! This module centralises panic formatting so logs and errors can present
//! consistent messages.

use std::any::Any;

/// Format a panic payload into a `String` using `Debug` formatting.
///
/// # Examples
/// ```
/// # use std::any::Any;
/// # use wireframe::panic::format_panic;
/// let payload: Box<dyn Any + Send> = Box::new("boom");
/// assert_eq!(format_panic(&payload), "Any { .. }");
/// ```
#[must_use]
pub fn format_panic(panic: &Box<dyn Any + Send>) -> String { format!("{panic:?}") }
