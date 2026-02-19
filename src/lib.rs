#![doc(html_root_url = "https://docs.rs/wireframe/latest")]
//! Public API for the `wireframe` library.
//!
//! This crate provides building blocks for asynchronous binary protocol
//! servers, including routing, middleware, and connection utilities.
//!
//! Public API discovery is tiered:
//! - Root exports stay intentionally small and stable.
//! - Domain modules hold detailed APIs (`app`, `server`, `client`, and so on).
//! - [`prelude`] offers optional convenience imports for common workflows.

extern crate self as wireframe;

pub mod app;
pub mod app_data_store;
pub mod byte_order;
#[cfg(not(loom))]
pub mod client;
pub mod codec;
pub mod error;
pub use error::{Result, WireframeError};
pub mod connection;
pub mod correlation;
#[cfg(not(loom))]
pub mod extractor;
mod fairness;
pub mod fragment;
pub mod frame;
pub mod hooks;
pub mod message;
pub mod message_assembler;
pub mod metrics;
pub mod middleware;
pub mod panic;
pub mod preamble;
pub mod prelude;
pub mod push;
#[cfg(not(loom))]
pub mod request;
pub mod response;
pub mod rewind_stream;
pub mod serializer;
#[cfg(not(loom))]
pub mod server;
pub mod session;
#[cfg(any(test, feature = "test-support"))]
pub mod test_helpers;
