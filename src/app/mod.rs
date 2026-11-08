//! Application builder and supporting types.
//!
//! Re-exports:
//! - [`WireframeApp`] and builder traits ([`Handler`], [`Middleware`], [`ConnectionSetup`],
//!   [`ConnectionTeardown`])
//! - Envelope primitives ([`Envelope`], [`Packet`], [`PacketParts`])
//! - Error handling types ([`SendError`], [`Result`])
//!
//! See the `examples/` directory for end-to-end usage.

mod builder;
mod builder_defaults;
mod codec_driver;
mod combined_codec;
pub(crate) mod connection_runtime;
mod envelope;
pub mod error;
pub mod fragment_utils;
mod fragmentation_state;
mod frame_handling;
mod inbound_handler;
mod lifecycle;
mod memory_budgets;
mod middleware_types;
mod outbound_encoding;
mod outbound_response;
mod prepared_app;

pub use builder::WireframeApp;
pub use envelope::{Envelope, Packet, PacketParts};
pub use error::{PrepareError, Result, SendError};
pub use lifecycle::{ConnectionSetup, ConnectionTeardown};
pub use memory_budgets::{BudgetBytes, MemoryBudgets};
pub use middleware_types::{Handler, Middleware};
#[cfg(feature = "test-support")]
#[doc(hidden)]
pub use outbound_encoding::benchmark_encode_message_frame;
pub use prepared_app::PreparedApp;
