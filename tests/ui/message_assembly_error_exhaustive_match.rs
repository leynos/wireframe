//! A downstream match written before ADR 015 no longer compiles.
//!
//! `MessageAssemblyError` is not `#[non_exhaustive]`, so adding the three
//! declared-total variants is a compile-time break. This fixture pins that
//! break: a match that names every variant from before the change fails to
//! compile until the new variants are handled.

use wireframe::message_assembler::MessageAssemblyError;

fn describe(error: MessageAssemblyError) -> &'static str {
    match error {
        MessageAssemblyError::Series(_) => "series",
        MessageAssemblyError::DuplicateFirstFrame { .. } => "duplicate first frame",
        MessageAssemblyError::MessageTooLarge { .. } => "too large",
        MessageAssemblyError::ConnectionBudgetExceeded { .. } => "connection budget",
        MessageAssemblyError::InFlightBudgetExceeded { .. } => "in-flight budget",
    }
}

fn main() {
    let _ = describe as fn(MessageAssemblyError) -> &'static str;
}
