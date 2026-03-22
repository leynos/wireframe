//! Fragment-aware in-memory driving helpers.

use std::{io, num::NonZeroUsize};

use tokio::io::DuplexStream;

use super::support::{
    DEFAULT_CAPACITY,
    TestSerializer,
    decode_frames_with_codec,
    drive_chunked_internal,
    drive_internal,
    encode_payloads_with_codec,
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

struct FragmentRequest<'a> {
    fragmenter: &'a Fragmenter,
    payload: Vec<u8>,
    route_id: u32,
    capacity: usize,
}

impl<'a> FragmentRequest<'a> {
    fn new(fragmenter: &'a Fragmenter, payload: Vec<u8>) -> Self {
        Self {
            fragmenter,
            payload,
            route_id: DEFAULT_FRAGMENT_ROUTE_ID,
            capacity: DEFAULT_CAPACITY,
        }
    }

    fn with_capacity(mut self, capacity: usize) -> Self {
        self.capacity = capacity;
        self
    }
}

async fn drive_fragments_internal<F, H, Fut>(
    handler: H,
    codec: &F,
    request: FragmentRequest<'_>,
) -> io::Result<Vec<F::Frame>>
where
    F: FrameCodec,
    H: FnOnce(DuplexStream) -> Fut,
    Fut: std::future::Future<Output = ()> + Send,
{
    let serialized_envelopes =
        fragment_and_encode(request.fragmenter, request.payload, request.route_id)?;
    let encoded = encode_payloads_with_codec(codec, serialized_envelopes)?;
    let raw = drive_internal(handler, encoded, request.capacity).await?;
    decode_frames_with_codec(codec, &raw)
}

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
    let frames = drive_fragments_internal(
        |server| run_owned_app(app, server),
        codec,
        FragmentRequest::new(fragmenter, payload).with_capacity(capacity),
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
    let frames = drive_fragments_internal(
        |server| async move { app.handle_connection(server).await },
        codec,
        FragmentRequest::new(fragmenter, payload),
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
    drive_fragments_internal(
        |server| run_owned_app(app, server),
        codec,
        FragmentRequest::new(fragmenter, payload),
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
    let serialized_envelopes = fragment_and_encode(fragmenter, payload, DEFAULT_FRAGMENT_ROUTE_ID)?;
    let encoded = encode_payloads_with_codec(codec, serialized_envelopes)?;
    let wire_bytes: Vec<u8> = encoded.into_iter().flatten().collect();
    let raw = drive_chunked_internal(
        |server| run_owned_app(app, server),
        wire_bytes,
        chunk_size,
        DEFAULT_CAPACITY,
    )
    .await?;
    let frames = decode_frames_with_codec(codec, &raw)?;
    Ok(extract_payloads::<F>(&frames))
}
