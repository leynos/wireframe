//! Fragment-aware in-memory driving helpers.
//!
//! These functions fragment a payload using a [`Fragmenter`], encode each
//! fragment via [`encode_fragment_payload`], wrap the results in codec frames,
//! and feed them through a [`WireframeApp`]. This allows test authors to
//! verify that fragmented messages survive the full application pipeline
//! without manually constructing fragment wire bytes.

use std::{io, num::NonZeroUsize};

use tokio::io::DuplexStream;
use wireframe::{
    app::{Packet, WireframeApp},
    codec::FrameCodec,
    fragment::{Fragmenter, encode_fragment_payload},
};

use super::{
    DEFAULT_CAPACITY,
    TestSerializer,
    codec_ext::{decode_frames_with_codec, encode_payloads_with_codec, extract_payloads},
    drive::drive_internal,
    partial_frame::drive_chunked_internal,
};

// ---------------------------------------------------------------------------
// Shared fragment encoding
// ---------------------------------------------------------------------------

/// Fragment `payload` and encode each fragment into a codec-ready payload.
fn fragment_and_encode(fragmenter: &Fragmenter, payload: Vec<u8>) -> io::Result<Vec<Vec<u8>>> {
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
            encode_fragment_payload(header, &body).map_err(|err| {
                io::Error::new(
                    io::ErrorKind::InvalidData,
                    format!("fragment encoding failed: {err}"),
                )
            })
        })
        .collect()
}

// ---------------------------------------------------------------------------
// Shared internal helper
// ---------------------------------------------------------------------------

/// Bundles the fragmenter, payload, and duplex buffer capacity needed by
/// [`drive_fragments_internal`].
struct FragmentRequest<'a> {
    /// Fragmenter that splits the payload into fragment frames.
    fragmenter: &'a Fragmenter,
    /// Raw payload bytes to fragment and feed.
    payload: Vec<u8>,
    /// Duplex stream buffer capacity.
    capacity: usize,
}

impl<'a> FragmentRequest<'a> {
    /// Create a request with the default duplex buffer capacity.
    fn new(fragmenter: &'a Fragmenter, payload: Vec<u8>) -> Self {
        Self {
            fragmenter,
            payload,
            capacity: DEFAULT_CAPACITY,
        }
    }

    /// Override the duplex buffer capacity.
    fn with_capacity(mut self, capacity: usize) -> Self {
        self.capacity = capacity;
        self
    }
}

/// Fragment, encode, transport, and decode — returning full codec frames.
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
    let fragment_payloads = fragment_and_encode(request.fragmenter, request.payload)?;
    let encoded = encode_payloads_with_codec(codec, fragment_payloads)?;
    let raw = drive_internal(handler, encoded, request.capacity).await?;
    decode_frames_with_codec(codec, raw)
}

// ---------------------------------------------------------------------------
// Payload-level drivers (return Vec<Vec<u8>>)
// ---------------------------------------------------------------------------

/// Fragment `payload`, encode each fragment into a codec frame, and drive
/// through `app`. Returns decoded response payloads.
///
/// # Errors
///
/// Returns any I/O, fragmentation, or codec error encountered during
/// encoding, transport, or decoding.
///
/// ```rust
/// # use std::num::NonZeroUsize;
/// # use wireframe::app::WireframeApp;
/// # use wireframe::codec::examples::HotlineFrameCodec;
/// # use wireframe::fragment::Fragmenter;
/// # use wireframe_testing::drive_with_fragments;
/// # async fn demo() -> std::io::Result<()> {
/// let codec = HotlineFrameCodec::new(4096);
/// let app = WireframeApp::new().expect("app").with_codec(codec.clone());
/// let fragmenter = Fragmenter::new(NonZeroUsize::new(20).unwrap());
/// let payloads = drive_with_fragments(app, &codec, &fragmenter, vec![0; 50]).await?;
/// # Ok(())
/// # }
/// ```
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
///
/// ```rust
/// # use std::num::NonZeroUsize;
/// # use wireframe::app::WireframeApp;
/// # use wireframe::codec::examples::HotlineFrameCodec;
/// # use wireframe::fragment::Fragmenter;
/// # use wireframe_testing::drive_with_fragments_with_capacity;
/// # async fn demo() -> std::io::Result<()> {
/// let codec = HotlineFrameCodec::new(4096);
/// let app = WireframeApp::new().expect("app").with_codec(codec.clone());
/// let fragmenter = Fragmenter::new(NonZeroUsize::new(20).unwrap());
/// let payloads =
///     drive_with_fragments_with_capacity(app, &codec, &fragmenter, vec![0; 50], 8192).await?;
/// # Ok(())
/// # }
/// ```
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
        |server| async move { app.handle_connection(server).await },
        codec,
        FragmentRequest::new(fragmenter, payload).with_capacity(capacity),
    )
    .await?;
    Ok(extract_payloads::<F>(&frames))
}

/// Fragment and feed through a mutable `app`.
///
/// The mutable reference allows the app instance to be reused across
/// successive calls.
///
/// # Errors
///
/// Returns any I/O, fragmentation, or codec error encountered during
/// encoding, transport, or decoding.
///
/// ```rust
/// # use std::num::NonZeroUsize;
/// # use wireframe::app::WireframeApp;
/// # use wireframe::codec::examples::HotlineFrameCodec;
/// # use wireframe::fragment::Fragmenter;
/// # use wireframe_testing::drive_with_fragments_mut;
/// # async fn demo() -> std::io::Result<()> {
/// let codec = HotlineFrameCodec::new(4096);
/// let mut app = WireframeApp::new().expect("app").with_codec(codec.clone());
/// let fragmenter = Fragmenter::new(NonZeroUsize::new(20).unwrap());
/// let payloads = drive_with_fragments_mut(&mut app, &codec, &fragmenter, vec![0; 50]).await?;
/// # Ok(())
/// # }
/// ```
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

// ---------------------------------------------------------------------------
// Frame-level driver (returns Vec<F::Frame>)
// ---------------------------------------------------------------------------

/// Fragment and feed through `app`, returning decoded response frames.
///
/// Unlike the payload-level drivers, this variant returns the full codec
/// frames so tests can inspect frame-level metadata.
///
/// # Errors
///
/// Returns any I/O, fragmentation, or codec error encountered during
/// encoding, transport, or decoding.
///
/// ```rust
/// # use std::num::NonZeroUsize;
/// # use wireframe::app::WireframeApp;
/// # use wireframe::codec::examples::HotlineFrameCodec;
/// # use wireframe::fragment::Fragmenter;
/// # use wireframe_testing::drive_with_fragment_frames;
/// # async fn demo() -> std::io::Result<()> {
/// let codec = HotlineFrameCodec::new(4096);
/// let app = WireframeApp::new().expect("app").with_codec(codec.clone());
/// let fragmenter = Fragmenter::new(NonZeroUsize::new(20).unwrap());
/// let frames = drive_with_fragment_frames(app, &codec, &fragmenter, vec![0; 50]).await?;
/// # Ok(())
/// # }
/// ```
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
        |server| async move { app.handle_connection(server).await },
        codec,
        FragmentRequest::new(fragmenter, payload),
    )
    .await
}

// ---------------------------------------------------------------------------
// Combined: fragment + chunked delivery
// ---------------------------------------------------------------------------

/// Fragment `payload` and feed the wire bytes in chunks of `chunk_size`,
/// exercising both fragmentation and partial-frame buffering simultaneously.
///
/// # Errors
///
/// Returns any I/O, fragmentation, or codec error encountered during
/// encoding, transport, or decoding.
///
/// ```rust
/// # use std::num::NonZeroUsize;
/// # use wireframe::app::WireframeApp;
/// # use wireframe::codec::examples::HotlineFrameCodec;
/// # use wireframe::fragment::Fragmenter;
/// # use wireframe_testing::drive_with_partial_fragments;
/// # async fn demo() -> std::io::Result<()> {
/// let codec = HotlineFrameCodec::new(4096);
/// let app = WireframeApp::new().expect("app").with_codec(codec.clone());
/// let fragmenter = Fragmenter::new(NonZeroUsize::new(20).unwrap());
/// let chunk = NonZeroUsize::new(3).expect("non-zero");
/// let payloads =
///     drive_with_partial_fragments(app, &codec, &fragmenter, vec![0; 50], chunk).await?;
/// # Ok(())
/// # }
/// ```
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
    let fragment_payloads = fragment_and_encode(fragmenter, payload)?;
    let encoded = encode_payloads_with_codec(codec, fragment_payloads)?;
    let wire_bytes: Vec<u8> = encoded.into_iter().flatten().collect();
    let raw = drive_chunked_internal(
        |server| async move { app.handle_connection(server).await },
        wire_bytes,
        chunk_size,
        DEFAULT_CAPACITY,
    )
    .await?;
    let frames = decode_frames_with_codec(codec, raw)?;
    Ok(extract_payloads::<F>(&frames))
}
