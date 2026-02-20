#![cfg(feature = "examples")]
//! Example Hotline framing codec wired into a `WireframeApp`.
//!
//! The Hotline header is 20 bytes with big-endian lengths and a transaction
//! identifier. The codec implementation lives in `wireframe::codec::examples`
//! so tests can exercise it directly.

use std::io;

use wireframe::{
    app::{Envelope, WireframeApp},
    codec::examples::HotlineFrameCodec,
    serializer::BincodeSerializer,
};

fn main() -> io::Result<()> {
    let app = WireframeApp::<BincodeSerializer, (), Envelope>::new()
        .map_err(|error| io::Error::other(error.to_string()))?
        .with_codec(HotlineFrameCodec::new(1024))
        .route(1, std::sync::Arc::new(|_: &Envelope| Box::pin(async {})))
        .map_err(|error| io::Error::other(error.to_string()))?;

    let _ = app;
    Ok(())
}
