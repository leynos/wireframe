//! Private support utilities shared across `wireframe::testkit`.

use std::{io, num::NonZeroUsize};

use bytes::{Bytes, BytesMut};
use tokio::io::{
    AsyncRead,
    AsyncReadExt,
    AsyncWrite,
    AsyncWriteExt,
    DuplexStream,
    ReadHalf,
    WriteHalf,
    duplex,
    split,
};
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

// ---------------------------------------------------------------------------
// Shared duplex driver
// ---------------------------------------------------------------------------

/// Wrap a server-side handler in a panic-catching future so panics surface as
/// `io::Error` instead of aborting the test harness.
async fn panic_guarded_server<F, Fut>(server_fn: F, server: DuplexStream) -> io::Result<()>
where
    F: FnOnce(DuplexStream) -> Fut,
    Fut: std::future::Future<Output = ()>,
{
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
}

/// Core driver that creates a duplex stream, splits it, and runs the server,
/// writer, and reader futures concurrently via `try_join!`.
///
/// Every in-memory driver in the testkit delegates here, passing strategy
/// closures for how bytes are written and read on the client side. The
/// strategies take ownership of their half of the stream so callers do not
/// need higher-ranked lifetime bounds.
pub(crate) async fn drive_with_strategies<F, Fut, WFn, WFut, RFn, RFut>(
    server_fn: F,
    capacity: usize,
    write_strategy: WFn,
    read_strategy: RFn,
) -> io::Result<Vec<u8>>
where
    F: FnOnce(DuplexStream) -> Fut,
    Fut: std::future::Future<Output = ()>,
    WFn: FnOnce(WriteHalf<DuplexStream>) -> WFut,
    WFut: std::future::Future<Output = io::Result<WriteHalf<DuplexStream>>>,
    RFn: FnOnce(ReadHalf<DuplexStream>) -> RFut,
    RFut: std::future::Future<Output = io::Result<Vec<u8>>>,
{
    let (client, server) = duplex(capacity);
    let (reader, writer) = split(client);

    let server_fut = panic_guarded_server(server_fn, server);

    let writer_fut = async {
        let mut writer = write_strategy(writer).await?;
        writer.shutdown().await?;
        io::Result::Ok(())
    };

    let reader_fut = read_strategy(reader);

    let ((), (), out) = tokio::try_join!(server_fut, writer_fut, reader_fut)?;
    Ok(out)
}

// ---------------------------------------------------------------------------
// Reusable write strategies
// ---------------------------------------------------------------------------

/// Write all frames sequentially with no pacing.
pub(crate) async fn write_frames(
    mut writer: impl AsyncWrite + Unpin,
    frames: &[Vec<u8>],
) -> io::Result<()> {
    for frame in frames {
        writer.write_all(frame).await?;
    }
    Ok(())
}

/// Write bytes in fixed-size chunks with no pacing.
pub(crate) async fn write_chunked(
    mut writer: impl AsyncWrite + Unpin,
    bytes: &[u8],
    chunk_size: NonZeroUsize,
) -> io::Result<()> {
    let total = bytes.len();
    let step = chunk_size.get();
    let mut offset = 0;
    while offset < total {
        let end = (offset + step).min(total);
        let chunk = bytes
            .get(offset..end)
            .ok_or_else(|| io::Error::other("chunk slice out of bounds"))?;
        writer.write_all(chunk).await?;
        offset = end;
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// Reusable read strategy
// ---------------------------------------------------------------------------

/// Read all bytes until EOF.
pub(crate) async fn read_all(mut reader: impl AsyncRead + Unpin) -> io::Result<Vec<u8>> {
    let mut buf = Vec::new();
    reader.read_to_end(&mut buf).await?;
    Ok(buf)
}

// ---------------------------------------------------------------------------
// High-level internal drivers (convenience wrappers)
// ---------------------------------------------------------------------------

pub(crate) async fn drive_internal<F, Fut>(
    server_fn: F,
    frames: Vec<Vec<u8>>,
    capacity: usize,
) -> io::Result<Vec<u8>>
where
    F: FnOnce(DuplexStream) -> Fut,
    Fut: std::future::Future<Output = ()> + Send,
{
    drive_with_strategies(
        server_fn,
        capacity,
        |mut writer| async {
            write_frames(&mut writer, &frames).await?;
            Ok(writer)
        },
        read_all,
    )
    .await
}

pub(crate) async fn drive_chunked_internal<F, Fut>(
    server_fn: F,
    wire_bytes: Vec<u8>,
    chunk_size: NonZeroUsize,
    capacity: usize,
) -> io::Result<Vec<u8>>
where
    F: FnOnce(DuplexStream) -> Fut,
    Fut: std::future::Future<Output = ()> + Send,
{
    drive_with_strategies(
        server_fn,
        capacity,
        |mut writer| async {
            write_chunked(&mut writer, &wire_bytes, chunk_size).await?;
            Ok(writer)
        },
        read_all,
    )
    .await
}

// ---------------------------------------------------------------------------
// Codec encode / decode
// ---------------------------------------------------------------------------

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

/// Shared encode → flatten → drive → decode pipeline used by partial-frame,
/// fragment, and slow-I/O helpers.
pub(crate) async fn drive_codec_roundtrip<F, H, Fut>(
    handler: H,
    codec: &F,
    payloads: Vec<Vec<u8>>,
    drive: impl AsyncFnOnce(H, Vec<u8>) -> io::Result<Vec<u8>>,
) -> io::Result<Vec<F::Frame>>
where
    F: FrameCodec,
    H: FnOnce(DuplexStream) -> Fut,
    Fut: std::future::Future<Output = ()> + Send,
{
    let encoded = encode_payloads_with_codec(codec, payloads)?;
    let wire_bytes: Vec<u8> = encoded.into_iter().flatten().collect();
    let raw = drive(handler, wire_bytes).await?;
    decode_frames_with_codec(codec, &raw)
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
