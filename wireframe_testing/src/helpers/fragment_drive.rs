//! Fragment-aware in-memory driving helpers.
//!
//! These functions fragment a payload using a [`Fragmenter`], encode each
//! fragment via [`encode_fragment_payload`], wrap the `FRAG`-prefixed bytes
//! inside a serialized [`Envelope`] packet, and feed the results through a
//! [`WireframeApp`] as codec frames. Wrapping in an `Envelope` ensures the
//! application's deserializer accepts the frames instead of accumulating
//! consecutive deserialization failures.

use std::{io, num::NonZeroUsize};

use tokio::io::DuplexStream;
use wireframe::{
    app::{Envelope, Packet, WireframeApp},
    codec::FrameCodec,
    fragment::{Fragmenter, encode_fragment_payload},
    serializer::{BincodeSerializer, Serializer},
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

/// Fragment `payload`, encode each fragment, and wrap the result in
/// serialized [`Envelope`] packets so the app's deserializer accepts them.
///
/// Each fragment is encoded via [`encode_fragment_payload`] into
/// `FRAG`-prefixed bytes, then placed as the `payload` field of an
/// `Envelope` and serialized with [`BincodeSerializer`]. This matches
/// the inbound path the application expects: deserialize the codec frame
/// payload as an `Envelope`, then hand it to the fragment reassembler.
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

// ---------------------------------------------------------------------------
// Shared internal helper
// ---------------------------------------------------------------------------

/// Default route identifier used when wrapping fragment payloads in
/// [`Envelope`] packets. Matches the route typically registered by test
/// applications built with `WireframeApp::route(1, ...)`.
const DEFAULT_FRAGMENT_ROUTE_ID: u32 = 1;

/// Bundles the fragmenter, payload, and duplex buffer capacity needed by
/// [`drive_fragments_internal`].
struct FragmentRequest<'a> {
    /// Fragmenter that splits the payload into fragment frames.
    fragmenter: &'a Fragmenter,
    /// Raw payload bytes to fragment and feed.
    payload: Vec<u8>,
    /// Route identifier for the wrapping [`Envelope`].
    route_id: u32,
    /// Duplex stream buffer capacity.
    capacity: usize,
}

impl<'a> FragmentRequest<'a> {
    /// Create a request with the default duplex buffer capacity.
    fn new(fragmenter: &'a Fragmenter, payload: Vec<u8>) -> Self {
        Self {
            fragmenter,
            payload,
            route_id: DEFAULT_FRAGMENT_ROUTE_ID,
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
    let serialized_envelopes =
        fragment_and_encode(request.fragmenter, request.payload, request.route_id)?;
    let encoded = encode_payloads_with_codec(codec, serialized_envelopes)?;
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
/// let fragmenter = Fragmenter::new(NonZeroUsize::new(20).expect("non-zero"));
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
/// let fragmenter = Fragmenter::new(NonZeroUsize::new(20).expect("non-zero"));
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
/// let fragmenter = Fragmenter::new(NonZeroUsize::new(20).expect("non-zero"));
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
/// let fragmenter = Fragmenter::new(NonZeroUsize::new(20).expect("non-zero"));
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
/// let fragmenter = Fragmenter::new(NonZeroUsize::new(20).expect("non-zero"));
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
    let serialized_envelopes = fragment_and_encode(fragmenter, payload, DEFAULT_FRAGMENT_ROUTE_ID)?;
    let encoded = encode_payloads_with_codec(codec, serialized_envelopes)?;
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
