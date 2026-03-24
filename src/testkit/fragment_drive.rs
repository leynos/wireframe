//! Fragment-aware in-memory driving helpers.

use std::{io, num::NonZeroUsize};

use super::support::{
    DEFAULT_CAPACITY,
    TestSerializer,
    drive_chunked_internal,
    drive_codec_roundtrip,
    drive_internal,
    extract_payloads,
    run_owned_app,
};
use crate::{
    app::{Envelope, Packet, WireframeApp},
    codec::FrameCodec,
    fragment::{Fragmenter, encode_fragment_payload},
    serializer::{BincodeSerializer, Serializer},
};

fn fragment_and_encode(
    fragmenter: &Fragmenter,
    payload: Vec<u8>,
    route_id: u32,
) -> io::Result<Vec<Vec<u8>>> {
    let batch = fragmenter.fragment_bytes(payload).map_err(|err| {
        io::Error::new(
            io::ErrorKind::InvalidData,
            format!("fragmentation failed: {err}"),
        )
    })?;

    batch
        .into_iter()
        .map(|frame| {
            let (header, body) = frame.into_parts();
            let frag_bytes = encode_fragment_payload(header, &body).map_err(|err| {
                io::Error::new(
                    io::ErrorKind::InvalidData,
                    format!("fragment encoding failed: {err}"),
                )
            })?;
            let env = Envelope::new(route_id, None, frag_bytes);
            BincodeSerializer.serialize(&env).map_err(|err| {
                io::Error::new(
                    io::ErrorKind::InvalidData,
                    format!("envelope serialization failed: {err}"),
                )
            })
        })
        .collect()
}

const DEFAULT_FRAGMENT_ROUTE_ID: u32 = 1;

/// Fragment `payload`, encode each fragment into a codec frame, and drive
/// through `app`.
///
/// # Errors
///
/// Returns any I/O, fragmentation, or codec error encountered during
/// encoding, transport, or decoding.
pub async fn drive_with_fragments<S, C, E, F>(
    app: WireframeApp<S, C, E, F>,
    codec: &F,
    fragmenter: &Fragmenter,
    payload: Vec<u8>,
) -> io::Result<Vec<Vec<u8>>>
where
    S: TestSerializer,
    C: Send + 'static,
    E: Packet,
    F: FrameCodec,
{
    drive_with_fragments_with_capacity(app, codec, fragmenter, payload, DEFAULT_CAPACITY).await
}

/// Fragment and feed with a duplex buffer of `capacity` bytes.
///
/// # Errors
///
/// Returns any I/O, fragmentation, or codec error encountered during
/// encoding, transport, or decoding.
#[expect(
    clippy::too_many_arguments,
    reason = "compatibility with the existing wireframe_testing helper API"
)]
pub async fn drive_with_fragments_with_capacity<S, C, E, F>(
    app: WireframeApp<S, C, E, F>,
    codec: &F,
    fragmenter: &Fragmenter,
    payload: Vec<u8>,
    capacity: usize,
) -> io::Result<Vec<Vec<u8>>>
where
    S: TestSerializer,
    C: Send + 'static,
    E: Packet,
    F: FrameCodec,
{
    let envelopes = fragment_and_encode(fragmenter, payload, DEFAULT_FRAGMENT_ROUTE_ID)?;
    let frames = drive_codec_roundtrip(
        |server| run_owned_app(app, server),
        codec,
        envelopes,
        |handler, wire_bytes| drive_internal(handler, vec![wire_bytes], capacity),
    )
    .await?;
    Ok(extract_payloads::<F>(&frames))
}

/// Fragment and feed through a mutable `app`.
///
/// # Errors
///
/// Returns any I/O, fragmentation, or codec error encountered during
/// encoding, transport, or decoding.
pub async fn drive_with_fragments_mut<S, C, E, F>(
    app: &mut WireframeApp<S, C, E, F>,
    codec: &F,
    fragmenter: &Fragmenter,
    payload: Vec<u8>,
) -> io::Result<Vec<Vec<u8>>>
where
    S: TestSerializer,
    C: Send + 'static,
    E: Packet,
    F: FrameCodec,
{
    let envelopes = fragment_and_encode(fragmenter, payload, DEFAULT_FRAGMENT_ROUTE_ID)?;
    let frames = drive_codec_roundtrip(
        |server| async move { app.handle_connection(server).await },
        codec,
        envelopes,
        |handler, wire_bytes| drive_internal(handler, vec![wire_bytes], DEFAULT_CAPACITY),
    )
    .await?;
    Ok(extract_payloads::<F>(&frames))
}

/// Fragment and feed through `app`, returning decoded response frames.
///
/// # Errors
///
/// Returns any I/O, fragmentation, or codec error encountered during
/// encoding, transport, or decoding.
pub async fn drive_with_fragment_frames<S, C, E, F>(
    app: WireframeApp<S, C, E, F>,
    codec: &F,
    fragmenter: &Fragmenter,
    payload: Vec<u8>,
) -> io::Result<Vec<F::Frame>>
where
    S: TestSerializer,
    C: Send + 'static,
    E: Packet,
    F: FrameCodec,
{
    let envelopes = fragment_and_encode(fragmenter, payload, DEFAULT_FRAGMENT_ROUTE_ID)?;
    drive_codec_roundtrip(
        |server| run_owned_app(app, server),
        codec,
        envelopes,
        |handler, wire_bytes| drive_internal(handler, vec![wire_bytes], DEFAULT_CAPACITY),
    )
    .await
}

/// Fragment `payload` and feed the wire bytes in chunks of `chunk_size`.
///
/// # Errors
///
/// Returns any I/O, fragmentation, or codec error encountered during
/// encoding, transport, or decoding.
#[expect(
    clippy::too_many_arguments,
    reason = "compatibility with the existing wireframe_testing helper API"
)]
pub async fn drive_with_partial_fragments<S, C, E, F>(
    app: WireframeApp<S, C, E, F>,
    codec: &F,
    fragmenter: &Fragmenter,
    payload: Vec<u8>,
    chunk_size: NonZeroUsize,
) -> io::Result<Vec<Vec<u8>>>
where
    S: TestSerializer,
    C: Send + 'static,
    E: Packet,
    F: FrameCodec,
{
    let envelopes = fragment_and_encode(fragmenter, payload, DEFAULT_FRAGMENT_ROUTE_ID)?;
    let frames = drive_codec_roundtrip(
        |server| run_owned_app(app, server),
        codec,
        envelopes,
        |handler, wire_bytes| {
            drive_chunked_internal(handler, wire_bytes, chunk_size, DEFAULT_CAPACITY)
        },
    )
    .await?;
    Ok(extract_payloads::<F>(&frames))
}
