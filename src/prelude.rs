//! Optional convenience imports for common Wireframe workflows.
//!
//! This module is intentionally small and focused on high-frequency types.
//! Prefer importing specialised APIs directly from their owning modules.
//!
//! # Examples
//!
//! ```rust,no_run
//! use wireframe::prelude::*;
//!
//! fn build() -> Result<WireframeApp> { WireframeApp::new() }
//! ```

pub use crate::{
    app::{Envelope, Handler, Middleware, WireframeApp},
    error::{Result, WireframeError},
    message::Message,
    response::Response,
    serializer::{BincodeSerializer, Serializer},
};
#[cfg(not(loom))]
pub use crate::{
    client::{ClientError, WireframeClient},
    request::{RequestBodyStream, RequestParts},
    server::{ServerError, WireframeServer},
};
