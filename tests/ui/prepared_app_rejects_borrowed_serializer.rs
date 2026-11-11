//! A serializer borrowed from a local cannot handle connections.
//!
//! The prepared application shares its serializer with the connection actor's
//! `'static` protocol hooks, so `handle_connection_result` requires
//! `S: 'static`. Preparing the application is still allowed; only the
//! connection boundary rejects the shorter lifetime.
use std::error::Error;

use wireframe::{
    app::{Envelope, WireframeApp},
    frame::FrameMetadata,
    message::{DecodeWith, EncodeWith},
    serializer::{BincodeSerializer, MessageCompatibilitySerializer, Serializer},
};

/// Serializer that borrows a value for the caller's lifetime.
struct BorrowedSerializer<'a>(&'a ());

impl Serializer for BorrowedSerializer<'_> {
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

impl MessageCompatibilitySerializer for BorrowedSerializer<'_> {}

impl FrameMetadata for BorrowedSerializer<'_> {
    type Frame = Envelope;
    type Error = std::io::Error;

    fn parse(&self, _src: &[u8]) -> Result<(Self::Frame, usize), Self::Error> {
        Err(std::io::Error::other("this fixture never parses a frame"))
    }
}

#[tokio::main]
async fn main() {
    let owner = ();
    let app = WireframeApp::<BincodeSerializer, (), Envelope>::default()
        .serializer(BorrowedSerializer(&owner));
    let prepared = app.prepare().await.expect("preparation should succeed");
    let (client, server) = tokio::io::duplex(64);
    drop(client);
    let _connection = prepared.handle_connection_result(server);
}
