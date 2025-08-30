//! Demonstrates routing based on frame metadata.
//!
//! Frames include a small header containing the message ID and flags,
//! which are used by `WireframeApp` to dispatch handlers.

use std::{io, sync::Arc};

use bytes::BytesMut;
use tokio::io::{AsyncWriteExt, duplex};
use tokio_util::codec::{Encoder, LengthDelimitedCodec};
use wireframe::{app::Envelope, frame::FrameMetadata, message::Message, serializer::Serializer};

type App = wireframe::app::WireframeApp<HeaderSerializer, (), Envelope>;

const MAX_FRAME: usize = 64 * 1024;

/// Frame format with a two-byte id, one-byte flags, and bincode payload.
#[derive(Default)]
struct HeaderSerializer;

impl Serializer for HeaderSerializer {
    fn serialize<M: Message>(
        &self,
        value: &M,
    ) -> Result<Vec<u8>, Box<dyn std::error::Error + Send + Sync>> {
        value
            .to_bytes()
            .map_err(|e| Box::new(e) as Box<dyn std::error::Error + Send + Sync>)
    }

    fn deserialize<M: Message>(
        &self,
        bytes: &[u8],
    ) -> Result<(M, usize), Box<dyn std::error::Error + Send + Sync>> {
        M::from_bytes(bytes).map_err(|e| Box::new(e) as Box<dyn std::error::Error + Send + Sync>)
    }
}

impl FrameMetadata for HeaderSerializer {
    type Frame = Envelope;
    type Error = io::Error;

    fn parse(&self, src: &[u8]) -> Result<(Envelope, usize), io::Error> {
        if src.len() < 3 {
            return Err(io::Error::new(io::ErrorKind::UnexpectedEof, "header"));
        }
        let id = u32::from(u16::from_be_bytes([src[0], src[1]]));
        // The third byte carries message flags. This example intentionally
        // ignores the flags, but a real protocol might parse and act on these
        // bits.
        let _ = src[2];
        let payload = src[3..].to_vec();
        // `parse` receives the complete frame because the length-delimited codec
        // ensures `src` contains exactly one message. Returning `src.len()` is
        // therefore correct for this demo.
        Ok((Envelope::new(id, None, payload), src.len()))
    }
}

#[derive(bincode::Decode, bincode::Encode)]
struct Ping;

#[tokio::main]
async fn main() -> io::Result<()> {
    let app = App::with_serializer(HeaderSerializer)
        .expect("failed to create app")
        .route(
            1,
            Arc::new(|_env: &Envelope| {
                Box::pin(async move {
                    tracing::info!("received ping message");
                })
            }),
        )
        .expect("failed to add ping route")
        .route(
            2,
            Arc::new(|_env: &Envelope| {
                Box::pin(async move {
                    tracing::info!("received pong message");
                })
            }),
        )
        .expect("failed to add pong route");

    let (mut client, server) = duplex(1024);
    let server_task = tokio::spawn(async move {
        app.handle_connection(server).await;
    });

    let payload = Ping.to_bytes().expect("failed to serialize Ping message");
    let mut frame = Vec::new();
    frame.extend_from_slice(&1u16.to_be_bytes());
    frame.push(0);
    frame.extend_from_slice(&payload);

    let mut codec = LengthDelimitedCodec::builder()
        .max_frame_length(MAX_FRAME) // 64 KiB cap for the example
        .new_codec();
    let mut bytes = BytesMut::with_capacity(frame.len() + 4); // +4 for the length prefix
    codec
        .encode(frame.into(), &mut bytes)
        .expect("failed to encode frame");

    client.write_all(&bytes).await?;
    client.shutdown().await?;

    server_task.await.expect("server task failed");
    Ok(())
}
