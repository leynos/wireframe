#![cfg(feature = "examples")]
//! Example `RESP` framing codec wired into a `WireframeApp`.
//!
//! This is a minimal RESP parser/encoder that supports a subset of types:
//! - Simple strings (`+OK`)
//! - Integers (`:1`)
//! - Bulk strings (`$3\r\nfoo`)
//! - Arrays (`*2\r\n$3\r\nfoo\r\n:1`)
//!
//! Wireframe payloads are carried in bulk strings. Other frame types are
//! decoded for completeness but do not expose a payload for routing.

use std::io;

#[path = "resp_codec_impl/mod.rs"]
mod resp_codec_impl;

pub use resp_codec_impl::{codec::RespFrameCodec, frame::RespFrame};
use wireframe::{
    BincodeSerializer,
    app::{Envelope, WireframeApp},
};

fn main() -> io::Result<()> {
    let app = WireframeApp::<BincodeSerializer, (), Envelope>::new()
        .map_err(|error| io::Error::other(error.to_string()))?
        .with_codec(RespFrameCodec::new(1024))
        .route(1, std::sync::Arc::new(|_: &Envelope| Box::pin(async {})))
        .map_err(|error| io::Error::other(error.to_string()))?;

    let _ = app;
    Ok(())
}
