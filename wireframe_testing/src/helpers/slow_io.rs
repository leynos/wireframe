//! Slow reader and writer simulation helpers for in-memory app driving.
//!
//! These helpers extend the existing duplex-based drivers with configurable
//! pacing on the client write side (slow writer) and client read side (slow
//! reader). They are intended for deterministic back-pressure tests.

use std::{io, num::NonZeroUsize, time::Duration};

use tokio::{
    io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt, DuplexStream, split},
    time::sleep,
};
use wireframe::{
    app::{Packet, WireframeApp},
    codec::{FrameCodec, LengthDelimitedFrameCodec},
};

use super::{
    DEFAULT_CAPACITY,
    TestSerializer,
    codec_ext::{decode_frames_with_codec, encode_payloads_with_codec, extract_payloads},
};

/// Maximum duplex capacity supported by the slow-I/O helpers.
pub const MAX_SLOW_IO_CAPACITY: usize = 1024 * 1024 * 10;

/// Pacing configuration for one I/O direction.
///
/// `chunk_size` controls how many bytes are transferred per operation, while
/// `delay` controls the pause between chunks.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct SlowIoPacing {
    /// Number of bytes transferred per chunk.
    pub chunk_size: NonZeroUsize,
    /// Delay inserted between successive chunks.
    pub delay: Duration,
}

impl SlowIoPacing {
    /// Create a pacing configuration for chunked transfers.
    pub fn new(chunk_size: NonZeroUsize, delay: Duration) -> Self { Self { chunk_size, delay } }
}

/// Shared configuration for slow-I/O app driving.
///
/// `writer_pacing` slows the client-to-server direction. `reader_pacing`
/// slows the server-to-client direction. When a pacing field is `None`, that
/// direction runs at full speed.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct SlowIoConfig {
    /// Optional pacing for bytes written into the app.
    pub writer_pacing: Option<SlowIoPacing>,
    /// Optional pacing for bytes read from the app.
    pub reader_pacing: Option<SlowIoPacing>,
    /// Duplex stream buffer capacity.
    pub capacity: usize,
}

impl Default for SlowIoConfig {
    fn default() -> Self {
        Self {
            writer_pacing: None,
            reader_pacing: None,
            capacity: DEFAULT_CAPACITY,
        }
    }
}

fn validate_pacing_chunk_size(
    pacing: Option<SlowIoPacing>,
    direction: &str,
    capacity: usize,
) -> io::Result<()> {
    if let Some(p) = pacing {
        if p.chunk_size.get() > capacity {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                format!(
                    "{direction} chunk size {} must not exceed capacity {}",
                    p.chunk_size.get(),
                    capacity
                ),
            ));
        }
    }
    Ok(())
}

impl SlowIoConfig {
    /// Create a config using the default duplex capacity and no pacing.
    pub fn new() -> Self { Self::default() }

    /// Set the pacing for bytes written into the app.
    #[must_use]
    pub fn with_writer_pacing(mut self, pacing: SlowIoPacing) -> Self {
        self.writer_pacing = Some(pacing);
        self
    }

    /// Set the pacing for bytes read from the app.
    #[must_use]
    pub fn with_reader_pacing(mut self, pacing: SlowIoPacing) -> Self {
        self.reader_pacing = Some(pacing);
        self
    }

    /// Set the duplex stream capacity.
    #[must_use]
    pub fn with_capacity(mut self, capacity: usize) -> Self {
        self.capacity = capacity;
        self
    }

    fn validate(self) -> io::Result<Self> {
        if self.capacity == 0 {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "capacity must be greater than zero",
            ));
        }
        if self.capacity > MAX_SLOW_IO_CAPACITY {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                format!("capacity must not exceed {MAX_SLOW_IO_CAPACITY} bytes"),
            ));
        }
        validate_pacing_chunk_size(self.writer_pacing, "writer", self.capacity)?;
        validate_pacing_chunk_size(self.reader_pacing, "reader", self.capacity)?;
        Ok(self)
    }
}

async fn pause_between_chunks(delay: Duration, should_pause: bool) {
    if should_pause && !delay.is_zero() {
        sleep(delay).await;
    }
}

async fn write_with_optional_pacing<W>(
    writer: &mut W,
    bytes: &[u8],
    pacing: Option<SlowIoPacing>,
) -> io::Result<()>
where
    W: AsyncWrite + Unpin,
{
    match pacing {
        None => writer.write_all(bytes).await,
        Some(pacing) => {
            let step = pacing.chunk_size.get();
            let total = bytes.len();
            let mut offset = 0;
            while offset < total {
                let end = (offset + step).min(total);
                let chunk = bytes
                    .get(offset..end)
                    .ok_or_else(|| io::Error::other("writer chunk slice out of bounds"))?;
                writer.write_all(chunk).await?;
                offset = end;
                pause_between_chunks(pacing.delay, offset < total).await;
            }
            Ok(())
        }
    }
}

async fn read_with_optional_pacing<R>(
    reader: &mut R,
    pacing: Option<SlowIoPacing>,
) -> io::Result<Vec<u8>>
where
    R: AsyncRead + Unpin,
{
    match pacing {
        None => {
            let mut out = Vec::new();
            reader.read_to_end(&mut out).await?;
            Ok(out)
        }
        Some(pacing) => {
            let mut out = Vec::new();
            let mut should_pause_before_read = false;
            let mut buf = vec![0; pacing.chunk_size.get()];
            loop {
                pause_between_chunks(pacing.delay, should_pause_before_read).await;
                let read = reader.read(&mut buf).await?;
                if read == 0 {
                    break;
                }
                let chunk = buf
                    .get(..read)
                    .ok_or_else(|| io::Error::other("reader chunk slice out of bounds"))?;
                out.extend_from_slice(chunk);
                should_pause_before_read = true;
            }
            Ok(out)
        }
    }
}

async fn drive_slow_internal<F, Fut>(
    server_fn: F,
    wire_bytes: Vec<u8>,
    config: SlowIoConfig,
) -> io::Result<Vec<u8>>
where
    F: FnOnce(DuplexStream) -> Fut,
    Fut: std::future::Future<Output = ()>,
{
    let config = config.validate()?;
    let (client, server) = tokio::io::duplex(config.capacity);
    let (mut reader, mut writer) = split(client);

    let server_fut = async {
        use futures::FutureExt as _;
        let result = std::panic::AssertUnwindSafe(server_fn(server))
            .catch_unwind()
            .await;
        match result {
            Ok(()) => Ok(()),
            Err(panic) => {
                let panic_msg = wireframe::panic::format_panic(&panic);
                Err(io::Error::other(format!("server task failed: {panic_msg}")))
            }
        }
    };

    let writer_fut = async {
        write_with_optional_pacing(&mut writer, &wire_bytes, config.writer_pacing).await?;
        writer.shutdown().await?;
        io::Result::Ok(())
    };

    let reader_fut = read_with_optional_pacing(&mut reader, config.reader_pacing);

    let ((), (), out) = tokio::try_join!(server_fut, writer_fut, reader_fut)?;
    Ok(out)
}

fn encode_length_delimited_payloads(payloads: Vec<Vec<u8>>) -> io::Result<Vec<u8>> {
    let codec = LengthDelimitedFrameCodec::new(DEFAULT_CAPACITY);
    let frames = encode_payloads_with_codec(&codec, payloads)?;
    Ok(frames.into_iter().flatten().collect())
}

/// Drive `app` with pre-framed bytes using optional slow writer and reader
/// pacing.
///
/// ```rust
/// # use std::{num::NonZeroUsize, time::Duration};
/// # use wireframe::app::WireframeApp;
/// # use wireframe_testing::{
/// #     drive_with_slow_frames, encode_frame, new_test_codec, SlowIoConfig, SlowIoPacing,
/// # };
/// # async fn demo() -> std::io::Result<()> {
/// let app =
///     WireframeApp::new().map_err(|error| std::io::Error::other(format!("app init: {error}")))?;
/// let mut codec = new_test_codec(4096);
/// let frame = encode_frame(&mut codec, vec![1, 2, 3])?;
/// let config = SlowIoConfig::new().with_writer_pacing(SlowIoPacing::new(
///     NonZeroUsize::new(2).ok_or_else(|| std::io::Error::other("chunk size must be non-zero"))?,
///     Duration::from_millis(5),
/// ));
/// let out = drive_with_slow_frames(app, vec![frame], config).await?;
/// # let _ = out;
/// # Ok(())
/// # }
/// ```
pub async fn drive_with_slow_frames<S, C, E>(
    app: WireframeApp<S, C, E>,
    frames: Vec<Vec<u8>>,
    config: SlowIoConfig,
) -> io::Result<Vec<u8>>
where
    S: TestSerializer,
    C: Send + 'static,
    E: Packet,
{
    let wire_bytes: Vec<u8> = frames.into_iter().flatten().collect();
    drive_slow_internal(
        |server| async move { app.handle_connection(server).await },
        wire_bytes,
        config,
    )
    .await
}

/// Encode payloads with the default length-delimited codec and drive `app`
/// using optional slow writer and reader pacing.
pub async fn drive_with_slow_payloads<S, C, E>(
    app: WireframeApp<S, C, E>,
    payloads: Vec<Vec<u8>>,
    config: SlowIoConfig,
) -> io::Result<Vec<u8>>
where
    S: TestSerializer,
    C: Send + 'static,
    E: Packet,
{
    let wire_bytes = encode_length_delimited_payloads(payloads)?;
    drive_slow_internal(
        |server| async move { app.handle_connection(server).await },
        wire_bytes,
        config,
    )
    .await
}

/// Drive `app` with codec-encoded payloads using optional slow I/O pacing and
/// return decoded response payloads.
///
/// ```rust
/// # use std::{num::NonZeroUsize, time::Duration};
/// # use wireframe::{
/// #     app::{Envelope, WireframeApp},
/// #     serializer::{BincodeSerializer, Serializer},
/// # };
/// # use wireframe::codec::examples::HotlineFrameCodec;
/// # use wireframe_testing::{
/// #     drive_with_slow_codec_payloads, SlowIoConfig, SlowIoPacing,
/// # };
/// # async fn demo() -> std::io::Result<()> {
/// let codec = HotlineFrameCodec::new(4096);
/// let app = WireframeApp::new()
///     .map_err(|error| std::io::Error::other(format!("app init: {error}")))?
///     .with_codec(codec.clone());
/// let config = SlowIoConfig::new().with_reader_pacing(SlowIoPacing::new(
///     NonZeroUsize::new(32)
///         .ok_or_else(|| std::io::Error::other("chunk size must be non-zero"))?,
///     Duration::from_millis(5),
/// ));
/// let request = BincodeSerializer
///     .serialize(&Envelope::new(1, Some(7), vec![1]))
///     .map_err(|error| std::io::Error::other(format!("serialize: {error}")))?;
/// let out = drive_with_slow_codec_payloads(app, &codec, vec![request], config).await?;
/// # let _ = out;
/// # Ok(())
/// # }
/// ```
pub async fn drive_with_slow_codec_payloads<S, C, E, F>(
    app: WireframeApp<S, C, E, F>,
    codec: &F,
    payloads: Vec<Vec<u8>>,
    config: SlowIoConfig,
) -> io::Result<Vec<Vec<u8>>>
where
    S: TestSerializer,
    C: Send + 'static,
    E: Packet,
    F: FrameCodec,
{
    let frames = drive_with_slow_codec_frames(app, codec, payloads, config).await?;
    Ok(extract_payloads::<F>(&frames))
}

/// Drive `app` with codec-encoded payloads using optional slow I/O pacing and
/// return decoded response frames.
pub async fn drive_with_slow_codec_frames<S, C, E, F>(
    app: WireframeApp<S, C, E, F>,
    codec: &F,
    payloads: Vec<Vec<u8>>,
    config: SlowIoConfig,
) -> io::Result<Vec<F::Frame>>
where
    S: TestSerializer,
    C: Send + 'static,
    E: Packet,
    F: FrameCodec,
{
    let encoded = encode_payloads_with_codec(codec, payloads)?;
    let wire_bytes: Vec<u8> = encoded.into_iter().flatten().collect();
    let raw = drive_slow_internal(
        |server| async move { app.handle_connection(server).await },
        wire_bytes,
        config,
    )
    .await?;
    decode_frames_with_codec(codec, raw)
}
