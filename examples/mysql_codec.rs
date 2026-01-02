#![cfg(feature = "examples")]
//! Example `MySQL` framing codec wired into a `WireframeApp`.
//!
//! `MySQL` packets use a 4-byte header: 3-byte little-endian payload length and
//! a 1-byte sequence number. The codec implementation lives in
//! `wireframe::codec::examples` so tests can exercise it directly.

use wireframe::{
    BincodeSerializer,
    app::{Envelope, WireframeApp},
    codec::examples::MysqlFrameCodec,
};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let app = WireframeApp::<BincodeSerializer, (), Envelope>::new()?
        .with_codec(MysqlFrameCodec::new(1024))
        .route(1, std::sync::Arc::new(|_: &Envelope| Box::pin(async {})))?;

    let _ = app;
    Ok(())
}
