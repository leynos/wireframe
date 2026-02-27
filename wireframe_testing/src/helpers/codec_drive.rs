//! Codec-aware in-memory driving helpers.
//!
//! These functions extend the frame-oriented drivers in [`super::drive`] with
//! automatic encoding and decoding through an arbitrary [`FrameCodec`]. Test
//! authors pass raw payloads and receive decoded payloads (or frames) without
//! manually constructing encoders or decoders.

use std::io;

use tokio::io::DuplexStream;
use wireframe::{
    app::{Packet, WireframeApp},
    codec::FrameCodec,
};

use super::{
    DEFAULT_CAPACITY,
    TestSerializer,
    codec_ext::{decode_frames_with_codec, encode_payloads_with_codec, extract_payloads},
    drive::drive_internal,
};

// ---------------------------------------------------------------------------
// Shared internal helper
// ---------------------------------------------------------------------------

/// Encode payloads, drive the server handler, and decode response frames.
///
/// Every public driver in this module delegates to this function. Keeping the
/// encode → transport → decode pipeline in one place prevents behavioural
/// drift between the owned and mutable variants.
async fn drive_codec_frames_internal<F, H, Fut>(
    handler: H,
    codec: &F,
    payloads: Vec<Vec<u8>>,
    capacity: usize,
) -> io::Result<Vec<F::Frame>>
where
    F: FrameCodec,
    H: FnOnce(DuplexStream) -> Fut,
    Fut: std::future::Future<Output = ()> + Send,
{
    let encoded = encode_payloads_with_codec(codec, payloads)?;
    let raw = drive_internal(handler, encoded, capacity).await?;
    decode_frames_with_codec(codec, raw)
}

// ---------------------------------------------------------------------------
// Payload-level drivers (return Vec<Vec<u8>>)
// ---------------------------------------------------------------------------

/// Drive `app` with payloads encoded by `codec` and return decoded response
/// payloads.
///
/// Each input payload is wrapped and encoded using the codec, sent through an
/// in-memory duplex stream, and the server's response bytes are decoded back
/// into payload byte vectors.
///
/// # Errors
///
/// Returns any I/O or codec error encountered during encoding, transport, or
/// decoding.
///
/// ```rust
/// # use wireframe::app::WireframeApp;
/// # use wireframe::codec::examples::HotlineFrameCodec;
/// # use wireframe_testing::drive_with_codec_payloads;
/// # async fn demo() -> std::io::Result<()> {
/// let codec = HotlineFrameCodec::new(4096);
/// let app = WireframeApp::new().expect("app").with_codec(codec.clone());
/// let payloads = drive_with_codec_payloads(app, &codec, vec![vec![1]]).await?;
/// # Ok(())
/// # }
/// ```
pub async fn drive_with_codec_payloads<S, C, E, F>(
    app: WireframeApp<S, C, E, F>,
    codec: &F,
    payloads: Vec<Vec<u8>>,
) -> io::Result<Vec<Vec<u8>>>
where
    S: TestSerializer,
    C: Send + 'static,
    E: Packet,
    F: FrameCodec,
{
    drive_with_codec_payloads_with_capacity(app, codec, payloads, DEFAULT_CAPACITY).await
}

/// Drive `app` with payloads using a duplex buffer of `capacity` bytes.
///
/// # Errors
///
/// Returns any I/O or codec error encountered during encoding, transport, or
/// decoding.
///
/// ```rust
/// # use wireframe::app::WireframeApp;
/// # use wireframe::codec::examples::HotlineFrameCodec;
/// # use wireframe_testing::drive_with_codec_payloads_with_capacity;
/// # async fn demo() -> std::io::Result<()> {
/// let codec = HotlineFrameCodec::new(4096);
/// let app = WireframeApp::new().expect("app").with_codec(codec.clone());
/// let payloads =
///     drive_with_codec_payloads_with_capacity(app, &codec, vec![vec![1]], 8192).await?;
/// # Ok(())
/// # }
/// ```
pub async fn drive_with_codec_payloads_with_capacity<S, C, E, F>(
    app: WireframeApp<S, C, E, F>,
    codec: &F,
    payloads: Vec<Vec<u8>>,
    capacity: usize,
) -> io::Result<Vec<Vec<u8>>>
where
    S: TestSerializer,
    C: Send + 'static,
    E: Packet,
    F: FrameCodec,
{
    let frames = drive_with_codec_frames_with_capacity(app, codec, payloads, capacity).await?;
    Ok(extract_payloads::<F>(&frames))
}

/// Drive a mutable `app` with payloads encoded by `codec`.
///
/// The mutable reference allows the app instance to be reused across
/// successive calls.
///
/// # Errors
///
/// Returns any I/O or codec error encountered during encoding, transport, or
/// decoding.
///
/// ```rust
/// # use wireframe::app::WireframeApp;
/// # use wireframe::codec::examples::HotlineFrameCodec;
/// # use wireframe_testing::drive_with_codec_payloads_mut;
/// # async fn demo() -> std::io::Result<()> {
/// let codec = HotlineFrameCodec::new(4096);
/// let mut app = WireframeApp::new().expect("app").with_codec(codec.clone());
/// let payloads = drive_with_codec_payloads_mut(&mut app, &codec, vec![vec![1]]).await?;
/// # Ok(())
/// # }
/// ```
pub async fn drive_with_codec_payloads_mut<S, C, E, F>(
    app: &mut WireframeApp<S, C, E, F>,
    codec: &F,
    payloads: Vec<Vec<u8>>,
) -> io::Result<Vec<Vec<u8>>>
where
    S: TestSerializer,
    C: Send + 'static,
    E: Packet,
    F: FrameCodec,
{
    drive_with_codec_payloads_with_capacity_mut(app, codec, payloads, DEFAULT_CAPACITY).await
}

/// Drive a mutable `app` with payloads using a duplex buffer of `capacity`
/// bytes.
///
/// # Errors
///
/// Returns any I/O or codec error encountered during encoding, transport, or
/// decoding.
///
/// ```rust
/// # use wireframe::app::WireframeApp;
/// # use wireframe::codec::examples::HotlineFrameCodec;
/// # use wireframe_testing::drive_with_codec_payloads_with_capacity_mut;
/// # async fn demo() -> std::io::Result<()> {
/// let codec = HotlineFrameCodec::new(4096);
/// let mut app = WireframeApp::new().expect("app").with_codec(codec.clone());
/// let payloads =
///     drive_with_codec_payloads_with_capacity_mut(&mut app, &codec, vec![vec![1]], 8192).await?;
/// # Ok(())
/// # }
/// ```
pub async fn drive_with_codec_payloads_with_capacity_mut<S, C, E, F>(
    app: &mut WireframeApp<S, C, E, F>,
    codec: &F,
    payloads: Vec<Vec<u8>>,
    capacity: usize,
) -> io::Result<Vec<Vec<u8>>>
where
    S: TestSerializer,
    C: Send + 'static,
    E: Packet,
    F: FrameCodec,
{
    let frames = drive_codec_frames_internal(
        |server| async move { app.handle_connection(server).await },
        codec,
        payloads,
        capacity,
    )
    .await?;
    Ok(extract_payloads::<F>(&frames))
}

// ---------------------------------------------------------------------------
// Frame-level drivers (return Vec<F::Frame>)
// ---------------------------------------------------------------------------

/// Drive `app` with payloads and return decoded response frames.
///
/// Unlike the payload-level drivers, this variant returns the full codec
/// frames so tests can inspect frame-level metadata such as transaction
/// identifiers or sequence numbers.
///
/// # Errors
///
/// Returns any I/O or codec error encountered during encoding, transport, or
/// decoding.
///
/// ```rust
/// # use wireframe::app::WireframeApp;
/// # use wireframe::codec::examples::HotlineFrameCodec;
/// # use wireframe_testing::drive_with_codec_frames;
/// # async fn demo() -> std::io::Result<()> {
/// let codec = HotlineFrameCodec::new(4096);
/// let app = WireframeApp::new().expect("app").with_codec(codec.clone());
/// let frames = drive_with_codec_frames(app, &codec, vec![vec![1]]).await?;
/// # Ok(())
/// # }
/// ```
pub async fn drive_with_codec_frames<S, C, E, F>(
    app: WireframeApp<S, C, E, F>,
    codec: &F,
    payloads: Vec<Vec<u8>>,
) -> io::Result<Vec<F::Frame>>
where
    S: TestSerializer,
    C: Send + 'static,
    E: Packet,
    F: FrameCodec,
{
    drive_with_codec_frames_with_capacity(app, codec, payloads, DEFAULT_CAPACITY).await
}

/// Drive `app` with payloads using a duplex buffer of `capacity` bytes and
/// return decoded response frames.
///
/// # Errors
///
/// Returns any I/O or codec error encountered during encoding, transport, or
/// decoding.
///
/// ```rust
/// # use wireframe::app::WireframeApp;
/// # use wireframe::codec::examples::HotlineFrameCodec;
/// # use wireframe_testing::drive_with_codec_frames_with_capacity;
/// # async fn demo() -> std::io::Result<()> {
/// let codec = HotlineFrameCodec::new(4096);
/// let app = WireframeApp::new().expect("app").with_codec(codec.clone());
/// let frames = drive_with_codec_frames_with_capacity(app, &codec, vec![vec![1]], 8192).await?;
/// # Ok(())
/// # }
/// ```
pub async fn drive_with_codec_frames_with_capacity<S, C, E, F>(
    app: WireframeApp<S, C, E, F>,
    codec: &F,
    payloads: Vec<Vec<u8>>,
    capacity: usize,
) -> io::Result<Vec<F::Frame>>
where
    S: TestSerializer,
    C: Send + 'static,
    E: Packet,
    F: FrameCodec,
{
    drive_codec_frames_internal(
        |server| async move { app.handle_connection(server).await },
        codec,
        payloads,
        capacity,
    )
    .await
}
