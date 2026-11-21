//! A prepared application owns a non-`Clone`, non-`Default`, `!Unpin` serializer.
//!
//! The prepared root is the sole owner of its serializer and connections borrow
//! it, so the connection boundary requires only `Send + Sync + 'static`. This
//! fixture pins those relaxed bounds by driving a connection with a serializer
//! that implements neither `Clone` nor `Default` and is deliberately not
//! `Unpin`.
use std::{error::Error, marker::PhantomPinned};

use wireframe::{
    app::{Envelope, PreparedApp, WireframeApp},
    codec::LengthDelimitedFrameCodec,
    frame::FrameMetadata,
    message::{DecodeWith, EncodeWith},
    serializer::{BincodeSerializer, MessageCompatibilitySerializer, Serializer},
};

/// Serializer that is neither `Clone` nor `Default` and cannot be moved once
/// pinned.
struct OwnedSerializer {
    /// Makes the serializer `!Unpin` without affecting its contents.
    _pin: PhantomPinned,
}

impl Serializer for OwnedSerializer {
    fn serialize<M>(&self, _value: &M) -> Result<Vec<u8>, Box<dyn Error + Send + Sync>>
    where
        M: EncodeWith<Self>,
    {
        todo!("this fixture never serializes a message")
    }

    fn deserialize<M>(&self, _bytes: &[u8]) -> Result<(M, usize), Box<dyn Error + Send + Sync>>
    where
        M: DecodeWith<Self>,
    {
        todo!("this fixture never deserializes a message")
    }
}

impl MessageCompatibilitySerializer for OwnedSerializer {}

impl FrameMetadata for OwnedSerializer {
    type Frame = Envelope;
    type Error = std::io::Error;

    fn parse(&self, _src: &[u8]) -> Result<(Self::Frame, usize), Self::Error> {
        Err(std::io::Error::other("this fixture never parses a frame"))
    }
}

/// Fail compilation unless `T` is safe to share across connection tasks.
fn assert_send_sync<T: Send + Sync>() {}

#[tokio::main]
async fn main() {
    assert_send_sync::<OwnedSerializer>();
    let app: PreparedApp<OwnedSerializer, (), Envelope> =
        WireframeApp::<BincodeSerializer, (), Envelope>::default()
            .serializer(OwnedSerializer {
                _pin: PhantomPinned,
            })
            .with_codec(LengthDelimitedFrameCodec::new(4096))
            .on_connection_setup(|| async {})
            .expect("setup registration should succeed")
            .prepare()
            .await
            .expect("preparation should succeed");

    let (client, server) = tokio::io::duplex(64);
    drop(client);
    app.handle_connection_result(server)
        .await
        .expect("an empty connection should finish cleanly");
}