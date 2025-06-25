#![doc(html_root_url = "https://docs.rs/wireframe/latest")]
//! Public API for the `wireframe` library.
//!
//! This crate provides building blocks for asynchronous binary protocol
//! servers, including routing, middleware, and connection utilities.

pub mod app;
pub mod serializer;
pub use serializer::{BincodeSerializer, Serializer};
pub mod extractor;
pub mod frame;
pub mod message;
pub mod middleware;
pub mod preamble;
pub mod push;
pub mod response;
pub mod rewind_stream;
pub mod server;

pub use response::{FrameStream, Response, WireframeError};
