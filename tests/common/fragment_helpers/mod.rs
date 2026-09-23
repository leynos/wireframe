//! Shared helpers for fragment transport integration tests.
//!
//! Provides a stable facade over focused helper modules for configuration,
//! fragment envelope construction, framed transport, test app construction,
//! and assertions.

#[path = "app.rs"]
mod app;
#[path = "assertions.rs"]
mod assertions;
#[path = "config.rs"]
mod config;
#[path = "envelopes.rs"]
mod envelopes;
#[path = "errors.rs"]
mod errors;
#[path = "transport.rs"]
mod transport;

pub use app::{make_app, make_handler, spawn_app};
pub use assertions::assert_handler_observed;
pub use config::{fragmentation_config, fragmentation_config_with_timeout};
pub use envelopes::{build_envelopes, fragment_envelope};
pub use errors::{TestError, TestResult};
pub use transport::{read_reassembled_response, read_response_payload, send_envelopes};

/// Default route ID used in fragmentation tests.
pub const ROUTE_ID: u32 = 42;
/// Default correlation ID used in fragmentation tests.
pub const CORRELATION: Option<u64> = Some(7);
