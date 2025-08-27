//! Application layer surface: builder, connection handling, envelope and error types.
//!
//! This module curates and re-exports the primary application APIs so downstream
//! crates can `use wireframe::app::*` to access:
//! - [`WireframeApp`] and builder traits ([`Handler`], [`Middleware`], [`ConnectionSetup`],
//!   [`ConnectionTeardown`])
//! - Envelope primitives ([`Envelope`], [`Packet`], [`PacketParts`])
//! - Error and result types ([`WireframeError`], [`SendError`], [`Result`])
//!
//! See the `examples/` directory for end-to-end usage.

mod builder;
mod connection;
mod envelope;
mod error;

pub use builder::{ConnectionSetup, ConnectionTeardown, Handler, Middleware, WireframeApp};
pub use envelope::{Envelope, Packet, PacketParts};
pub use error::{Result, SendError, WireframeError};
