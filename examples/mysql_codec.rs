#![cfg(feature = "examples")]
//! Example `MySQL` framing codec wired into a `WireframeApp`.
//!
//! `MySQL` packets use a 4-byte header: 3-byte little-endian payload length and
//! a 1-byte sequence number. The codec implementation lives in
//! `wireframe::codec::examples` so tests can exercise it directly.

use std::io;

use wireframe::{
    app::{Envelope, WireframeApp},
    codec::examples::MysqlFrameCodec,
    serializer::BincodeSerializer,
};

fn main() -> io::Result<()> {
    let app = WireframeApp::<BincodeSerializer, (), Envelope>::new()
        .map_err(|error| io::Error::other(error.to_string()))?
        .with_codec(MysqlFrameCodec::new(1024))
        .route(1, std::sync::Arc::new(|_: &Envelope| Box::pin(async {})))
        .map_err(|error| io::Error::other(error.to_string()))?;

    let _ = app;
    Ok(())
}
