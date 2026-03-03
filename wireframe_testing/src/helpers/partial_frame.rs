//! Chunked-write in-memory driving helpers.
//!
//! These functions extend the frame-oriented drivers in [`super::drive`] with
//! configurable chunk sizes, forcing the codec decoder on the server side to
//! buffer partial frames across reads. This exercises realistic network
//! conditions where a single codec frame may arrive across multiple TCP reads.

use std::{io, num::NonZeroUsize};

use tokio::io::{AsyncReadExt, AsyncWriteExt, DuplexStream, duplex};
use wireframe::{
    app::{Packet, WireframeApp},
    codec::FrameCodec,
};

use super::{
    DEFAULT_CAPACITY,
    TestSerializer,
    codec_ext::{decode_frames_with_codec, encode_payloads_with_codec, extract_payloads},
};

/// Drive a server function by writing `wire_bytes` in chunks of
/// `chunk_size` bytes, forcing partial-frame reads on the server side.
///
/// This mirrors [`super::drive::drive_internal`] but replaces per-frame
/// `write_all` calls with a chunked iteration that slices the concatenated
/// wire bytes into fixed-size pieces.
///
/// ```rust
/// use std::num::NonZeroUsize;
///
/// use tokio::io::{AsyncWriteExt, DuplexStream};
/// use wireframe_testing::helpers::partial_frame::drive_chunked_internal;
///
/// async fn echo(mut s: DuplexStream) { let _ = s.write_all(&[1, 2]).await; }
///
/// # async fn demo() -> std::io::Result<()> {
/// let out = drive_chunked_internal(echo, vec![0], NonZeroUsize::new(1).unwrap(), 64).await?;
/// assert_eq!(out, [1, 2]);
/// # Ok(())
/// # }
/// ```
pub(super) async fn drive_chunked_internal<F, Fut>(
    server_fn: F,
    wire_bytes: Vec<u8>,
    chunk_size: NonZeroUsize,
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
                let panic_msg = wireframe::panic::format_panic(&panic);
                Err(io::Error::new(
                    io::ErrorKind::Other,
                    format!("server task failed: {panic_msg}"),
                ))
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

// ---------------------------------------------------------------------------
// Shared internal helper
// ---------------------------------------------------------------------------

/// Encode payloads, chunk the wire bytes, drive the server, and decode
/// response frames.
async fn drive_partial_frames_internal<F, H, Fut>(
    handler: H,
    codec: &F,
    payloads: Vec<Vec<u8>>,
    chunk_size: NonZeroUsize,
    capacity: usize,
) -> io::Result<Vec<F::Frame>>
where
    F: FrameCodec,
    H: FnOnce(DuplexStream) -> Fut,
    Fut: std::future::Future<Output = ()> + Send,
{
    let encoded = encode_payloads_with_codec(codec, payloads)?;
    let wire_bytes: Vec<u8> = encoded.into_iter().flatten().collect();
    let raw = drive_chunked_internal(handler, wire_bytes, chunk_size, capacity).await?;
    decode_frames_with_codec(codec, raw)
}

// ---------------------------------------------------------------------------
// Payload-level drivers (return Vec<Vec<u8>>)
// ---------------------------------------------------------------------------

/// Drive `app` with payloads encoded by `codec`, writing wire bytes in
/// chunks of `chunk_size` to exercise partial-frame buffering.
///
/// Each input payload is encoded through the codec, and the resulting wire
/// bytes are concatenated and written `chunk_size` bytes at a time. The
/// server's responses are decoded and returned as payload byte vectors.
///
/// # Errors
///
/// Returns any I/O or codec error encountered during encoding, transport, or
/// decoding.
///
/// ```rust
/// # use std::num::NonZeroUsize;
/// # use wireframe::app::WireframeApp;
/// # use wireframe::codec::examples::HotlineFrameCodec;
/// # use wireframe_testing::drive_with_partial_frames;
/// # async fn demo() -> std::io::Result<()> {
/// let codec = HotlineFrameCodec::new(4096);
/// let app = WireframeApp::new().expect("app").with_codec(codec.clone());
/// let chunk = NonZeroUsize::new(1).expect("non-zero");
/// let payloads = drive_with_partial_frames(app, &codec, vec![vec![1]], chunk).await?;
/// # Ok(())
/// # }
/// ```
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

/// Drive `app` with payloads in chunks using a duplex buffer of `capacity`
/// bytes.
///
/// # Errors
///
/// Returns any I/O or codec error encountered during encoding, transport, or
/// decoding.
///
/// ```rust
/// # use std::num::NonZeroUsize;
/// # use wireframe::app::WireframeApp;
/// # use wireframe::codec::examples::HotlineFrameCodec;
/// # use wireframe_testing::drive_with_partial_frames_with_capacity;
/// # async fn demo() -> std::io::Result<()> {
/// let codec = HotlineFrameCodec::new(4096);
/// let app = WireframeApp::new().expect("app").with_codec(codec.clone());
/// let chunk = NonZeroUsize::new(3).expect("non-zero");
/// let payloads =
///     drive_with_partial_frames_with_capacity(app, &codec, vec![vec![1]], chunk, 8192).await?;
/// # Ok(())
/// # }
/// ```
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
        |server| async move { app.handle_connection(server).await },
        codec,
        payloads,
        chunk_size,
        capacity,
    )
    .await?;
    Ok(extract_payloads::<F>(&frames))
}

/// Drive a mutable `app` with payloads in chunks of `chunk_size`.
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
/// # use std::num::NonZeroUsize;
/// # use wireframe::app::WireframeApp;
/// # use wireframe::codec::examples::HotlineFrameCodec;
/// # use wireframe_testing::drive_with_partial_frames_mut;
/// # async fn demo() -> std::io::Result<()> {
/// let codec = HotlineFrameCodec::new(4096);
/// let mut app = WireframeApp::new().expect("app").with_codec(codec.clone());
/// let chunk = NonZeroUsize::new(5).expect("non-zero");
/// let payloads = drive_with_partial_frames_mut(&mut app, &codec, vec![vec![1]], chunk).await?;
/// # Ok(())
/// # }
/// ```
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
        chunk_size,
        DEFAULT_CAPACITY,
    )
    .await?;
    Ok(extract_payloads::<F>(&frames))
}

// ---------------------------------------------------------------------------
// Frame-level driver (returns Vec<F::Frame>)
// ---------------------------------------------------------------------------

/// Drive `app` with payloads in chunks and return decoded response frames.
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
/// # use std::num::NonZeroUsize;
/// # use wireframe::app::WireframeApp;
/// # use wireframe::codec::examples::HotlineFrameCodec;
/// # use wireframe_testing::drive_with_partial_codec_frames;
/// # async fn demo() -> std::io::Result<()> {
/// let codec = HotlineFrameCodec::new(4096);
/// let app = WireframeApp::new().expect("app").with_codec(codec.clone());
/// let chunk = NonZeroUsize::new(2).expect("non-zero");
/// let frames = drive_with_partial_codec_frames(app, &codec, vec![vec![1]], chunk).await?;
/// # Ok(())
/// # }
/// ```
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
        |server| async move { app.handle_connection(server).await },
        codec,
        payloads,
        chunk_size,
        DEFAULT_CAPACITY,
    )
    .await
}
