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

#[path = "resp_codec_impl/mod.rs"]
mod resp_codec_impl;

pub use resp_codec_impl::{codec::RespFrameCodec, frame::RespFrame};
use wireframe::{
    BincodeSerializer,
    app::{Envelope, WireframeApp},
};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let app = WireframeApp::<BincodeSerializer, (), Envelope>::new()?
        .with_codec(RespFrameCodec::new(1024))
        .route(1, std::sync::Arc::new(|_: &Envelope| Box::pin(async {})))?;

    let _ = app;
    Ok(())
}
