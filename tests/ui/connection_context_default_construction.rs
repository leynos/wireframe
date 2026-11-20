//! `ConnectionContext::default()` is the supported construction path.
//!
//! A context built this way describes a connection with no server-assigned
//! identity, which is what a standalone actor sees. The accessors must be
//! callable and report that absence without exposing any field.
use wireframe::hooks::ConnectionContext;

fn main() {
    let context = ConnectionContext::default();
    assert_eq!(context.connection_id(), None);
    assert_eq!(context.peer_addr(), None);
}