//! A serializer without `Default` may still configure a frame codec.
//!
//! `WireframeApp::with_codec` used to require `S: Default`, which forced a
//! custom serializer to implement a trait it never needed. This fixture pins
//! the relaxed bound: the serializer is installed through `serializer`, and
//! the codec is then replaced.
use std::error::Error;

use wireframe::{
    app::{Envelope, WireframeApp},
    codec::LengthDelimitedFrameCodec,
    message::{DecodeWith, EncodeWith},
    serializer::{BincodeSerializer, Serializer},
};

/// Serializer deliberately lacking a `Default` implementation.
struct NoDefaultSerializer;

impl Serializer for NoDefaultSerializer {
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

fn main() {
    let _app: WireframeApp<NoDefaultSerializer, (), Envelope, LengthDelimitedFrameCodec> =
        WireframeApp::<BincodeSerializer, (), Envelope>::default()
            .serializer(NoDefaultSerializer)
            .with_codec(LengthDelimitedFrameCodec::new(4096));
}
