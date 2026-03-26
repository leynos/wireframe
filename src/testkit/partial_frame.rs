//! Chunked-write in-memory driving helpers.

use std::{io, num::NonZeroUsize};

use super::support::{
    DEFAULT_CAPACITY,
    TestSerializer,
    drive_chunked_internal,
    drive_codec_roundtrip,
    extract_payloads,
    run_owned_app,
};
use crate::{
    app::{Packet, WireframeApp},
    codec::FrameCodec,
};

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
    let frames = drive_codec_roundtrip(
        |server| run_owned_app(app, server),
        codec,
        payloads,
        |handler, wire_bytes| drive_chunked_internal(handler, wire_bytes, chunk_size, capacity),
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
    let frames = drive_codec_roundtrip(
        |server| async move { app.handle_connection(server).await },
        codec,
        payloads,
        |handler, wire_bytes| {
            drive_chunked_internal(handler, wire_bytes, chunk_size, DEFAULT_CAPACITY)
        },
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
    drive_codec_roundtrip(
        |server| run_owned_app(app, server),
        codec,
        payloads,
        |handler, wire_bytes| {
            drive_chunked_internal(handler, wire_bytes, chunk_size, DEFAULT_CAPACITY)
        },
    )
    .await
}
