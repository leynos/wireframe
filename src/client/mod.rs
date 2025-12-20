//! Client runtime for wireframe connections.
//!
//! This module provides a configurable client runtime that mirrors the
//! server's framing and serialization layers.

mod builder;
mod config;
mod error;
mod runtime;

pub use builder::WireframeClientBuilder;
pub use config::{ClientCodecConfig, SocketOptions};
pub use error::ClientError;
pub use runtime::WireframeClient;

#[cfg(test)]
mod tests;
