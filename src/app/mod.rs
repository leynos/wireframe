//! Application builder and supporting types.
//!
//! Re-exports:
//! - [`WireframeApp`] and builder traits ([`Handler`], [`Middleware`], [`ConnectionSetup`],
//!   [`ConnectionTeardown`])
//! - Envelope primitives ([`Envelope`], [`Packet`], [`PacketParts`])
//! - Error handling types ([`WireframeError`], [`SendError`], [`Result`])
//!
//! See the `examples/` directory for end-to-end usage.

mod builder;
mod connection;
mod envelope;
pub mod error;

pub use builder::{ConnectionSetup, ConnectionTeardown, Handler, Middleware, WireframeApp};
pub use envelope::{Envelope, Packet, PacketParts};
pub use error::{Result, SendError, WireframeError};
