#![doc(html_root_url = "https://docs.rs/wireframe/latest")]
//! Public API for the `wireframe` library.
//!
//! This crate provides building blocks for asynchronous binary protocol
//! servers, including routing, middleware, and connection utilities.

pub mod app;
pub mod config;
pub use config::SerializationFormat;
pub mod extractor;
pub mod frame;
pub mod message;
pub mod middleware;
pub mod preamble;
pub mod rewind_stream;
pub mod server;
