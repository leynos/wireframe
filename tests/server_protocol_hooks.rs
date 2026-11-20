//! End-to-end protocol hook dispatch through a real [`WireframeServer`].
//!
//! Each hook a [`WireframeProtocol`] exposes must run on a server connection at
//! a documented point, so these tests observe a real socket rather than an
//! actor or pipeline in isolation. The scenarios are grouped by concern:
//!
//! - [`lifecycle`] covers hook order for a streamed response, a clean close, and a rejected inbound
//!   frame.
//! - [`eof`] covers the three shapes a peer can abandon a frame in.
//! - [`consuming_decoder`] covers what a custom decoder leaves for `on_eof`.
//! - [`pushes`] covers server-initiated pushes and handle closure.
//! - [`stream_push`] covers a push queued while a streamed response is in flight.
//! - [`setup`] covers a push queued at setup and setup ordering against the first read.
//!
//! [`WireframeProtocol`]: wireframe::hooks::WireframeProtocol
//! [`WireframeServer`]: wireframe::server::WireframeServer
#![cfg(not(loom))]
#![forbid(unsafe_code)]

#[path = "common/hook_recorder.rs"]
mod hook_recorder;

// Integration-test roots resolve submodules beside themselves in `tests/`,
// so each scenario module names its file explicitly.
#[path = "server_protocol_hooks/consuming_decoder.rs"]
mod consuming_decoder;
#[path = "server_protocol_hooks/eof.rs"]
mod eof;
#[path = "server_protocol_hooks/harness.rs"]
mod harness;
#[path = "server_protocol_hooks/lifecycle.rs"]
mod lifecycle;
#[path = "server_protocol_hooks/pushes.rs"]
mod pushes;
#[path = "server_protocol_hooks/setup.rs"]
mod setup;
#[path = "server_protocol_hooks/stream_push.rs"]
mod stream_push;
