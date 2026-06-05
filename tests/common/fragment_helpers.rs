//! Shared helpers for fragment transport integration tests.
//!
//! Provides a stable facade over focused helper modules for configuration,
//! fragment envelope construction, framed transport, test app construction,
//! and assertions.

#[path = "fragment_helpers/app.rs"]
mod app;
#[path = "fragment_helpers/assertions.rs"]
mod assertions;
#[path = "fragment_helpers/config.rs"]
mod config;
#[path = "fragment_helpers/envelopes.rs"]
mod envelopes;
#[path = "fragment_helpers/errors.rs"]
mod errors;
#[path = "fragment_helpers/transport.rs"]
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
