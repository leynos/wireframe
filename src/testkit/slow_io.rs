//! Slow reader and writer simulation helpers for in-memory app driving.

use std::{io, num::NonZeroUsize, time::Duration};

use tokio::{
    io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt, DuplexStream, split},
    time::sleep,
};

use super::support::{
    DEFAULT_CAPACITY,
    TestSerializer,
    decode_frames_with_codec,
    encode_payloads_with_codec,
    extract_payloads,
    run_owned_app,
};
use crate::{
    app::{Packet, WireframeApp},
    codec::{FrameCodec, LengthDelimitedFrameCodec},
};

/// Maximum duplex capacity supported by the slow-I/O helpers.
pub const MAX_SLOW_IO_CAPACITY: usize = 1024 * 1024 * 10;

/// Pacing configuration for one I/O direction.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct SlowIoPacing {
    /// Number of bytes transferred per chunk.
    pub chunk_size: NonZeroUsize,
    /// Delay inserted between successive chunks.
    pub delay: Duration,
}

impl SlowIoPacing {
    /// Create a pacing configuration for chunked transfers.
    #[must_use]
    pub fn new(chunk_size: NonZeroUsize, delay: Duration) -> Self { Self { chunk_size, delay } }
}

/// Shared configuration for slow-I/O app driving.
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

impl SlowIoConfig {
    /// Create a config using the default duplex capacity and no pacing.
    #[must_use]
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

fn validate_pacing_chunk_size(
    pacing: Option<SlowIoPacing>,
    direction: &str,
    capacity: usize,
) -> io::Result<()> {
    if let Some(pacing) = pacing
        && pacing.chunk_size.get() > capacity
    {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            format!(
                "{direction} chunk size {} must not exceed capacity {}",
                pacing.chunk_size.get(),
                capacity
            ),
        ));
    }
    Ok(())
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
                let panic_msg = crate::panic::format_panic(&panic);
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

/// Drive `app` with pre-framed bytes using optional slow writer and reader pacing.
///
/// # Errors
///
/// Returns any I/O or configuration error encountered while driving the
/// in-memory transport.
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
    drive_slow_internal(|server| run_owned_app(app, server), wire_bytes, config).await
}

/// Encode payloads with the default length-delimited codec and drive `app`.
///
/// # Errors
///
/// Returns any I/O or configuration error encountered while driving the
/// in-memory transport.
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
    drive_slow_internal(|server| run_owned_app(app, server), wire_bytes, config).await
}

/// Drive `app` with codec-encoded payloads using optional slow I/O pacing.
///
/// # Errors
///
/// Returns any I/O, configuration, or codec error encountered during
/// encoding, transport, or decoding.
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
///
/// # Errors
///
/// Returns any I/O, configuration, or codec error encountered during
/// encoding, transport, or decoding.
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
    let raw = drive_slow_internal(|server| run_owned_app(app, server), wire_bytes, config).await?;
    decode_frames_with_codec(codec, &raw)
}
