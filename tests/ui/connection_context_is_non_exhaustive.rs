//! `ConnectionContext` no longer has a unit-struct value expression.
//!
//! The context became `#[non_exhaustive]` with private fields so the server can
//! add connection-scoped state without breaking callbacks. Both constructions
//! the former unit struct accepted must stop compiling: a bare value expression
//! and an empty struct literal. Applications must reach it through
//! `ConnectionContext::default()` instead.
use wireframe::hooks::ConnectionContext;

fn main() {
    let _unit = ConnectionContext;
    let _literal = ConnectionContext {};
}