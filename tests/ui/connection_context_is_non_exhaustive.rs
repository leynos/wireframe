//! `ConnectionContext` no longer has a unit-struct value expression.
//!
//! The context became `#[non_exhaustive]` with private fields so the server can
//! add connection-scoped state without breaking callbacks. Both forms that
//! worked against the former unit struct must stop compiling, which tells an
//! application to use `ConnectionContext::default()` instead.
use wireframe::hooks::ConnectionContext;

fn main() {
    let _unit = ConnectionContext;
    let _literal = ConnectionContext {
        connection_id: None,
        peer_addr: None,
    };
}