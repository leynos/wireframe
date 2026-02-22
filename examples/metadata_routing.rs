//! Demonstrates routing based on frame metadata.
//!
//! Frames include a small header containing the message ID and flags,
//! which are used by `WireframeApp` to dispatch handlers.

use std::{io, sync::Arc};

use bytes::BytesMut;
use tokio::io::{AsyncWriteExt, duplex};
use tokio_util::codec::Encoder;
use wireframe::{
    app::Envelope,
    byte_order::{read_network_u16, write_network_u16},
    frame::FrameMetadata,
    message::{DecodeWith, DeserializeContext, EncodeWith, Message},
    serializer::Serializer,
};

type App = wireframe::app::WireframeApp<HeaderSerializer, (), Envelope>;

const MAX_FRAME: usize = 64 * 1024;

/// Frame format with a two-byte id, one-byte flags, and bincode payload.
#[derive(Default)]
struct HeaderSerializer;

impl Serializer for HeaderSerializer {
    fn serialize<M>(&self, value: &M) -> Result<Vec<u8>, Box<dyn std::error::Error + Send + Sync>>
    where
        M: EncodeWith<Self>,
    {
        value.encode_with(self)
    }

    fn deserialize<M>(
        &self,
        bytes: &[u8],
    ) -> Result<(M, usize), Box<dyn std::error::Error + Send + Sync>>
    where
        M: DecodeWith<Self>,
    {
        M::decode_with(self, bytes, &DeserializeContext::empty())
    }
}

impl FrameMetadata for HeaderSerializer {
    type Frame = Envelope;
    type Error = io::Error;

    fn parse(&self, src: &[u8]) -> Result<(Envelope, usize), io::Error> {
        let id_bytes: [u8; 2] = src
            .get(..2)
            .ok_or_else(|| io::Error::new(io::ErrorKind::UnexpectedEof, "header"))?
            .try_into()
            .map_err(|_| io::Error::new(io::ErrorKind::InvalidData, "header id width"))?;

        // The third byte carries message flags. This example intentionally
        // ignores the flags, but a real protocol might parse and act on these
        // bits. We still validate its presence to avoid panics.
        let _flags = src
            .get(2)
            .copied()
            .ok_or_else(|| io::Error::new(io::ErrorKind::UnexpectedEof, "header flags"))?;

        // Only extract metadata here; defer payload handling to the serializer.
        // Header ID is defined as big-endian on the wire; read_network_u16 keeps
        // parsing deterministic across host architectures.
        let msg_id = read_network_u16(id_bytes);

        Ok((Envelope::new(u32::from(msg_id), None, Vec::new()), 3))
    }
}

#[derive(bincode::Decode, bincode::Encode)]
struct Ping;

#[tokio::main]
async fn main() -> io::Result<()> {
    let app = App::with_serializer(HeaderSerializer)
        .map_err(|error| io::Error::other(error.to_string()))?
        .buffer_capacity(MAX_FRAME)
        .route(
            1,
            Arc::new(|_env: &Envelope| {
                Box::pin(async move {
                    tracing::info!("received ping message");
                })
            }),
        )
        .map_err(|error| io::Error::other(error.to_string()))?
        .route(
            2,
            Arc::new(|_env: &Envelope| {
                Box::pin(async move {
                    tracing::info!("received pong message");
                })
            }),
        )
        .map_err(|error| io::Error::other(error.to_string()))?;

    let mut codec = app.length_codec();
    let (mut client, server) = duplex(1024);
    let server_task = tokio::spawn(async move { app.handle_connection_result(server).await });

    let payload = Ping.to_bytes().map_err(io::Error::other)?;
    let mut frame = Vec::new();
    // Frame header mandates big-endian message ID; write_network_u16 ensures
    // the generated example frame matches the on-wire format.
    let msg_id_bytes = write_network_u16(1);
    frame.extend_from_slice(&msg_id_bytes);
    frame.push(0);
    frame.extend_from_slice(&payload);
    let mut bytes = BytesMut::with_capacity(frame.len() + 4); // +4 for the length prefix
    codec.encode(frame.into(), &mut bytes)?;

    client.write_all(&bytes).await?;
    client.shutdown().await?;

    server_task.await.map_err(io::Error::other)??;
    Ok(())
}
