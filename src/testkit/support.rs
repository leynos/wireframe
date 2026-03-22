//! Private support utilities shared across `wireframe::testkit`.

use std::io;

use bytes::{Bytes, BytesMut};
use tokio::io::{AsyncReadExt, AsyncWriteExt, DuplexStream, duplex};
use tokio_util::codec::{Decoder, Encoder};

use crate::{
    app::{Envelope, Packet, WireframeApp},
    codec::FrameCodec,
    frame::FrameMetadata,
    serializer::{MessageCompatibilitySerializer, Serializer},
};

pub(crate) const DEFAULT_CAPACITY: usize = 4096;

/// Serializer bounds expected by the in-memory test harness.
#[doc(hidden)]
pub trait TestSerializer:
    Serializer
    + MessageCompatibilitySerializer
    + FrameMetadata<Frame = Envelope>
    + Send
    + Sync
    + 'static
{
}

impl<T> TestSerializer for T where
    T: Serializer
        + MessageCompatibilitySerializer
        + FrameMetadata<Frame = Envelope>
        + Send
        + Sync
        + 'static
{
}

pub(crate) async fn drive_internal<F, Fut>(
    server_fn: F,
    frames: Vec<Vec<u8>>,
    capacity: usize,
) -> io::Result<Vec<u8>>
where
    F: FnOnce(DuplexStream) -> Fut,
    Fut: std::future::Future<Output = ()> + Send,
{
    let (mut client, server) = duplex(capacity);

    let server_fut = async {
        use futures::FutureExt as _;
        let result = std::panic::AssertUnwindSafe(server_fn(server))
            .catch_unwind()
            .await;
        match result {
            Ok(()) => Ok(()),
            Err(panic) => {
                let panic_msg = crate::panic::format_panic(&panic);
                Err(io::Error::other(format!("server task failed: {panic_msg}")))
            }
        }
    };

    let client_fut = async {
        for frame in &frames {
            client.write_all(frame).await?;
        }
        client.shutdown().await?;

        let mut buf = Vec::new();
        client.read_to_end(&mut buf).await?;
        io::Result::Ok(buf)
    };

    let ((), buf) = tokio::try_join!(server_fut, client_fut)?;
    Ok(buf)
}

pub(crate) async fn drive_chunked_internal<F, Fut>(
    server_fn: F,
    wire_bytes: Vec<u8>,
    chunk_size: std::num::NonZeroUsize,
    capacity: usize,
) -> io::Result<Vec<u8>>
where
    F: FnOnce(DuplexStream) -> Fut,
    Fut: std::future::Future<Output = ()> + Send,
{
    let (mut client, server) = duplex(capacity);

    let server_fut = async {
        use futures::FutureExt as _;
        let result = std::panic::AssertUnwindSafe(server_fn(server))
            .catch_unwind()
            .await;
        match result {
            Ok(()) => Ok(()),
            Err(panic) => {
                let panic_msg = crate::panic::format_panic(&panic);
                Err(io::Error::other(format!("server task failed: {panic_msg}")))
            }
        }
    };

    let client_fut = async {
        let total = wire_bytes.len();
        let step = chunk_size.get();
        let mut offset = 0;
        while offset < total {
            let end = (offset + step).min(total);
            let chunk = wire_bytes
                .get(offset..end)
                .ok_or_else(|| io::Error::other("chunk slice out of bounds"))?;
            client.write_all(chunk).await?;
            offset = end;
        }
        client.shutdown().await?;

        let mut buf = Vec::new();
        client.read_to_end(&mut buf).await?;
        io::Result::Ok(buf)
    };

    let ((), buf) = tokio::try_join!(server_fut, client_fut)?;
    Ok(buf)
}

pub(crate) fn encode_payloads_with_codec<F: FrameCodec>(
    codec: &F,
    payloads: Vec<Vec<u8>>,
) -> io::Result<Vec<Vec<u8>>> {
    let mut encoder = codec.encoder();
    payloads
        .into_iter()
        .map(|payload| {
            let frame = codec.wrap_payload(Bytes::from(payload));
            let mut buf = BytesMut::new();
            encoder.encode(frame, &mut buf).map_err(|error| {
                io::Error::new(
                    io::ErrorKind::InvalidData,
                    format!("codec encode failed: {error}"),
                )
            })?;
            Ok(buf.to_vec())
        })
        .collect()
}

pub(crate) fn decode_frames_with_codec<F: FrameCodec>(
    codec: &F,
    bytes: &[u8],
) -> io::Result<Vec<F::Frame>> {
    let mut decoder = codec.decoder();
    let mut buf = BytesMut::from(bytes);
    let mut frames = Vec::new();
    while let Some(frame) = decoder.decode(&mut buf)? {
        frames.push(frame);
    }

    while let Some(frame) = decoder.decode_eof(&mut buf)? {
        frames.push(frame);
    }

    if !buf.is_empty() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            format!(
                "trailing {} byte(s) after decoding - possible truncated frame",
                buf.len()
            ),
        ));
    }

    Ok(frames)
}

pub(crate) fn extract_payloads<F: FrameCodec>(frames: &[F::Frame]) -> Vec<Vec<u8>> {
    frames
        .iter()
        .map(|frame| F::frame_payload(frame).to_vec())
        .collect()
}

pub(crate) async fn run_owned_app<S, C, E, F>(app: WireframeApp<S, C, E, F>, server: DuplexStream)
where
    S: TestSerializer,
    C: Send + 'static,
    E: Packet,
    F: FrameCodec,
{
    app.handle_connection(server).await;
}
