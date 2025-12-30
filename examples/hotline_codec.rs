#![cfg(feature = "examples")]
//! Example Hotline framing codec wired into a `WireframeApp`.
//!
//! The Hotline header is 20 bytes with big-endian lengths and a transaction
//! identifier. The codec implementation lives in `wireframe::codec::examples`
//! so tests can exercise it directly.

use wireframe::{
    BincodeSerializer,
    app::{Envelope, WireframeApp},
    codec::examples::HotlineFrameCodec,
};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let app = WireframeApp::<BincodeSerializer, (), Envelope>::new()?
        .with_codec(HotlineFrameCodec::new(1024))
        .route(1, std::sync::Arc::new(|_: &Envelope| Box::pin(async {})))?;

    let _ = app;
    Ok(())
}
