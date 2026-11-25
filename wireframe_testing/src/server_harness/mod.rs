//! Protocol-agnostic server lifecycle harness.
//!
//! [`spawn_wireframe_server`] accepts a fully configured but unbound
//! [`WireframeServer`](wireframe::server::WireframeServer), binds a reserved
//! loopback listener, waits for readiness, and returns a
//! [`RunningWireframeServer`]. The handle is not generic over the server's
//! application or protocol types, knows its address, and stops idempotently,
//! so a downstream crate can exercise its own serializer, codec, preamble, and
//! worker count without hand-rolling listener ownership.
//!
//! The helpers take over the readiness signal: the harness must know when
//! connection attempts are safe. `ready_signal` cannot be enforced, and the
//! server exposes no accessor that would reveal a caller-set sender, so a
//! sender installed before the call never fires. Do not configure
//! `ready_signal` on a server you pass here.
//!
//! # Examples
//!
//! ```rust
//! use wireframe::{app::WireframeApp, server::WireframeServer};
//! use wireframe_testing::{TestResult, spawn_wireframe_server};
//!
//! #[tokio::main(flavor = "current_thread")]
//! async fn main() -> TestResult<()> {
//!     let mut server = spawn_wireframe_server(WireframeServer::new(|| -> WireframeApp {
//!         WireframeApp::default()
//!     }))
//!     .await?;
//!
//!     assert!(server.local_addr().ip().is_loopback());
//!     server.shutdown().await?;
//!     Ok(())
//! }
//! ```

mod handle;
mod lifecycle;
mod options;
mod outcome;

pub(crate) use self::lifecycle::{bind_with_readiness, start_bound};
pub use self::{
    handle::RunningWireframeServer,
    lifecycle::{
        spawn_wireframe_server,
        spawn_wireframe_server_on,
        spawn_wireframe_server_with_options,
    },
    options::ServerHarnessOptions,
    outcome::ShutdownOutcome,
};
