//! A discarded `ServerTaskGuard` must be rejected.
//!
//! Dropping the guard aborts the server without awaiting it, so the task's
//! outcome is lost. The guard is `#[must_use]` to force a caller to pick a
//! runner instead.
#![deny(unused_must_use)]

use wireframe::test_helpers::ServerTaskGuard;

fn main() {
    let server = tokio::spawn(async { 7_u32 });
    ServerTaskGuard::new(server);
}