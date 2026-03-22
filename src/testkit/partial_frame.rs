//! Chunked-write in-memory driving helpers.

use std::{io, num::NonZeroUsize};

use tokio::io::DuplexStream;

use super::support::{
    DEFAULT_CAPACITY,
    TestSerializer,
    decode_frames_with_codec,
    drive_chunked_internal,
    encode_payloads_with_codec,
    extract_payloads,
    run_owned_app,
};
use crate::{
    app::{Packet, WireframeApp},
    codec::FrameCodec,
};

#[derive(Debug, Clone, Copy)]
struct ChunkConfig {
    chunk_size: NonZeroUsize,
    capacity: usize,
}

impl ChunkConfig {
    fn new(chunk_size: NonZeroUsize) -> Self {
        Self {
            chunk_size,
            capacity: DEFAULT_CAPACITY,
        }
    }

    fn with_capacity(chunk_size: NonZeroUsize, capacity: usize) -> Self {
        Self {
            chunk_size,
            capacity,
        }
    }
}

async fn drive_partial_frames_internal<F, H, Fut>(
    handler: H,
    codec: &F,
    payloads: Vec<Vec<u8>>,
    config: ChunkConfig,
) -> io::Result<Vec<F::Frame>>
where
    F: FrameCodec,
    H: FnOnce(DuplexStream) -> Fut,
    Fut: std::future::Future<Output = ()> + Send,
{
    let encoded = encode_payloads_with_codec(codec, payloads)?;
    let wire_bytes: Vec<u8> = encoded.into_iter().flatten().collect();
    let raw =
        drive_chunked_internal(handler, wire_bytes, config.chunk_size, config.capacity).await?;
    decode_frames_with_codec(codec, &raw)
}

/// Drive `app` with payloads encoded by `codec`, writing wire bytes in chunks.
///
/// # Errors
///
/// Returns any I/O or codec error encountered during encoding, transport, or
/// decoding.
pub async fn drive_with_partial_frames<S, C, E, F>(
    app: WireframeApp<S, C, E, F>,
    codec: &F,
    payloads: Vec<Vec<u8>>,
    chunk_size: NonZeroUsize,
) -> io::Result<Vec<Vec<u8>>>
where
    S: TestSerializer,
    C: Send + 'static,
    E: Packet,
    F: FrameCodec,
{
    drive_with_partial_frames_with_capacity(app, codec, payloads, chunk_size, DEFAULT_CAPACITY)
        .await
}

/// Drive `app` with payloads in chunks using a duplex buffer of `capacity`.
///
/// # Errors
///
/// Returns any I/O or codec error encountered during encoding, transport, or
/// decoding.
#[expect(
    clippy::too_many_arguments,
    reason = "compatibility with the existing wireframe_testing helper API"
)]
pub async fn drive_with_partial_frames_with_capacity<S, C, E, F>(
    app: WireframeApp<S, C, E, F>,
    codec: &F,
    payloads: Vec<Vec<u8>>,
    chunk_size: NonZeroUsize,
    capacity: usize,
) -> io::Result<Vec<Vec<u8>>>
where
    S: TestSerializer,
    C: Send + 'static,
    E: Packet,
    F: FrameCodec,
{
    let frames = drive_partial_frames_internal(
        |server| run_owned_app(app, server),
        codec,
        payloads,
        ChunkConfig::with_capacity(chunk_size, capacity),
    )
    .await?;
    Ok(extract_payloads::<F>(&frames))
}

/// Drive a mutable `app` with payloads in chunks of `chunk_size`.
///
/// # Errors
///
/// Returns any I/O or codec error encountered during encoding, transport, or
/// decoding.
pub async fn drive_with_partial_frames_mut<S, C, E, F>(
    app: &mut WireframeApp<S, C, E, F>,
    codec: &F,
    payloads: Vec<Vec<u8>>,
    chunk_size: NonZeroUsize,
) -> io::Result<Vec<Vec<u8>>>
where
    S: TestSerializer,
    C: Send + 'static,
    E: Packet,
    F: FrameCodec,
{
    let frames = drive_partial_frames_internal(
        |server| async move { app.handle_connection(server).await },
        codec,
        payloads,
        ChunkConfig::new(chunk_size),
    )
    .await?;
    Ok(extract_payloads::<F>(&frames))
}

/// Drive `app` with payloads in chunks and return decoded response frames.
///
/// # Errors
///
/// Returns any I/O or codec error encountered during encoding, transport, or
/// decoding.
pub async fn drive_with_partial_codec_frames<S, C, E, F>(
    app: WireframeApp<S, C, E, F>,
    codec: &F,
    payloads: Vec<Vec<u8>>,
    chunk_size: NonZeroUsize,
) -> io::Result<Vec<F::Frame>>
where
    S: TestSerializer,
    C: Send + 'static,
    E: Packet,
    F: FrameCodec,
{
    drive_partial_frames_internal(
        |server| run_owned_app(app, server),
        codec,
        payloads,
        ChunkConfig::new(chunk_size),
    )
    .await
}
