//! End-to-end TCP coverage for streaming request bodies.
//!
//! Every scenario drives a real [`WireframeServer`] over a socket rather than
//! calling a connection API directly, so the tests cover the same path a
//! deployed server takes: accept, prepare, frame, dispatch, reply.
//!
//! - [`delivery`] proves a body split across frames reaches the handler while later frames are
//!   still unsent, that a one-frame message arrives as one chunk, and that a connection closed
//!   mid-body reports `UnexpectedEof`.
//! - [`back_pressure`] proves a handler that stops reading stalls the peer through the socket, and
//!   that a message parked behind a live body is still answered.
//! - [`middleware`] proves a streaming route bypasses the middleware chain while a buffered route
//!   on the same connection still runs it.
//!
//! The client frames with [`LengthDelimitedCodec`] rather than hand-built
//! length prefixes, so the tests speak exactly the wire format the runtime
//! decodes.
//!
//! [`WireframeServer`]: wireframe::server::WireframeServer
//! [`LengthDelimitedCodec`]: tokio_util::codec::LengthDelimitedCodec

#![forbid(unsafe_code)]

#[path = "../common/tagging.rs"]
mod tagging;

mod back_pressure;
mod delivery;
mod middleware;
mod support;
