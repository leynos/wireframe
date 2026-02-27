//! Outbound streaming send methods for the wireframe client.
//!
//! These methods enable sending large request bodies as multiple frames by
//! reading from an [`AsyncRead`] source and emitting protocol-framed chunks.
//! This is the outbound counterpart to inbound streaming via
//! [`RequestBodyStream`](crate::request::RequestBodyStream).
//!
//! Each emitted frame consists of caller-provided `frame_header` bytes
//! followed by up to `chunk_size` bytes read from the body source.
//! The helper handles chunking, flushing, timeouts, and error hook
//! integration — keeping header semantics protocol-defined while
//! Wireframe provides the shared transport plumbing.
//!
//! See ADR 0002 § 4 for the design rationale.

use std::{io, time::Duration};

use bytes::Bytes;
use futures::SinkExt;
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWriteExt};

use super::{ClientError, runtime::ClientStream};
use crate::serializer::Serializer;

/// Configuration for outbound streaming sends.
///
/// Controls chunk sizing and timeout behaviour for
/// [`WireframeClient::send_streaming`](super::WireframeClient::send_streaming).
///
/// # Examples
///
/// ```
/// use std::time::Duration;
///
/// use wireframe::client::SendStreamingConfig;
///
/// let config = SendStreamingConfig::default()
///     .with_chunk_size(4096)
///     .with_timeout(Duration::from_secs(30));
/// ```
#[derive(Clone, Copy, Debug, Default)]
pub struct SendStreamingConfig {
    chunk_size: Option<usize>,
    timeout: Option<Duration>,
}

impl SendStreamingConfig {
    /// Set the maximum number of body bytes per frame.
    ///
    /// When not set, the chunk size is derived as
    /// `max_frame_length - frame_header.len()`.
    ///
    /// If the configured value exceeds the available frame capacity
    /// (after accounting for the header), it is silently clamped.
    ///
    /// # Examples
    ///
    /// ```
    /// use wireframe::client::SendStreamingConfig;
    ///
    /// let config = SendStreamingConfig::default().with_chunk_size(8192);
    /// ```
    #[must_use]
    pub fn with_chunk_size(mut self, size: usize) -> Self {
        self.chunk_size = Some(size);
        self
    }

    /// Set a timeout for the entire streaming send operation.
    ///
    /// If the timeout elapses, `send_streaming` returns
    /// `std::io::ErrorKind::TimedOut` and stops emitting frames.
    /// Any frames already sent remain sent.
    ///
    /// # Examples
    ///
    /// ```
    /// use std::time::Duration;
    ///
    /// use wireframe::client::SendStreamingConfig;
    ///
    /// let config = SendStreamingConfig::default().with_timeout(Duration::from_secs(10));
    /// ```
    #[must_use]
    pub fn with_timeout(mut self, duration: Duration) -> Self {
        self.timeout = Some(duration);
        self
    }

    /// Return the configured chunk size, if any.
    #[must_use]
    pub const fn chunk_size(&self) -> Option<usize> { self.chunk_size }

    /// Return the configured timeout, if any.
    #[must_use]
    pub const fn timeout(&self) -> Option<Duration> { self.timeout }
}

/// Outcome of a successful streaming send operation.
///
/// Reports the number of frames emitted during the operation. Useful for
/// logging and instrumentation.
///
/// # Examples
///
/// ```
/// use wireframe::client::SendStreamingOutcome;
///
/// let outcome = SendStreamingOutcome::new(5);
/// assert_eq!(outcome.frames_sent(), 5);
/// ```
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SendStreamingOutcome {
    frames_sent: u64,
}

impl SendStreamingOutcome {
    /// Construct an outcome with the given frame count.
    #[must_use]
    pub const fn new(frames_sent: u64) -> Self { Self { frames_sent } }

    /// Return the number of frames emitted during the operation.
    #[must_use]
    pub const fn frames_sent(&self) -> u64 { self.frames_sent }
}

impl<S, T, C> super::WireframeClient<S, T, C>
where
    S: Serializer + Send + Sync,
    T: ClientStream,
{
    /// Send a large body as multiple frames, reading from an async source.
    ///
    /// Each emitted frame consists of `frame_header` followed by up to
    /// `chunk_size` bytes read from `body_reader`. The chunk size defaults
    /// to `max_frame_length - frame_header.len()` when not explicitly
    /// configured via [`SendStreamingConfig::with_chunk_size`].
    ///
    /// # Timeout semantics
    ///
    /// When a timeout is configured, it wraps the entire operation. If the
    /// timeout elapses, no further frames are emitted and
    /// `std::io::ErrorKind::TimedOut` is returned. Any frames already sent
    /// remain sent — callers must assume the operation may have been
    /// partially successful.
    ///
    /// # Errors
    ///
    /// Returns [`ClientError`] if:
    ///
    /// - The frame header is as long as or longer than `max_frame_length` (no room for body data).
    /// - A transport I/O error occurs during a frame send.
    /// - The configured timeout elapses.
    ///
    /// The error hook is invoked before any error is returned.
    ///
    /// # Examples
    ///
    /// ```no_run
    /// use std::{net::SocketAddr, time::Duration};
    ///
    /// use wireframe::client::{ClientError, SendStreamingConfig, WireframeClient};
    ///
    /// # #[tokio::main]
    /// # async fn main() -> Result<(), ClientError> {
    /// let addr: SocketAddr = "127.0.0.1:9000".parse().expect("valid socket address");
    /// let mut client = WireframeClient::builder().connect(addr).await?;
    ///
    /// let header = b"\x01\x02";
    /// let body = b"hello, streaming world!";
    /// let config = SendStreamingConfig::default()
    ///     .with_chunk_size(8)
    ///     .with_timeout(Duration::from_secs(5));
    ///
    /// let outcome = client.send_streaming(header, &body[..], config).await?;
    /// println!("sent {} frames", outcome.frames_sent());
    /// # Ok(())
    /// # }
    /// ```
    pub async fn send_streaming<R: AsyncRead + Unpin>(
        &mut self,
        frame_header: &[u8],
        body_reader: R,
        config: SendStreamingConfig,
    ) -> Result<SendStreamingOutcome, ClientError> {
        let effective_chunk_size = match effective_chunk_size(
            frame_header.len(),
            self.codec_config.max_frame_length_value(),
            &config,
        ) {
            Ok(size) => size,
            Err(err) => {
                self.invoke_error_hook(&err).await;
                return Err(err);
            }
        };

        match config.timeout {
            Some(duration) => match tokio::time::timeout(
                duration,
                self.send_streaming_inner(frame_header, body_reader, effective_chunk_size),
            )
            .await
            {
                Ok(result) => result,
                Err(_elapsed) => {
                    // Shut down the write side so buffered codec state cannot
                    // leak into subsequent operations on this connection.
                    let _ = self.framed.get_mut().shutdown().await;
                    let err = ClientError::from(io::Error::new(
                        io::ErrorKind::TimedOut,
                        "streaming send timed out",
                    ));
                    self.invoke_error_hook(&err).await;
                    Err(err)
                }
            },
            None => {
                self.send_streaming_inner(frame_header, body_reader, effective_chunk_size)
                    .await
            }
        }
    }

    /// Core loop: read chunks from `body_reader` and send framed packets.
    async fn send_streaming_inner<R: AsyncRead + Unpin>(
        &mut self,
        frame_header: &[u8],
        mut body_reader: R,
        chunk_size: usize,
    ) -> Result<SendStreamingOutcome, ClientError> {
        let header_len = frame_header.len();
        let mut buf = vec![0u8; chunk_size];
        let mut frames_sent: u64 = 0;

        loop {
            let n = match read_chunk(&mut body_reader, &mut buf).await {
                ReadChunk::Eof => break,
                ReadChunk::Bytes(n) => n,
                ReadChunk::Err(e) => {
                    self.invoke_error_hook(&e).await;
                    return Err(e);
                }
            };

            let chunk = buf.get(..n).ok_or_else(|| {
                ClientError::from(io::Error::new(
                    io::ErrorKind::InvalidData,
                    "read returned more bytes than buffer length",
                ))
            })?;
            let mut frame = Vec::with_capacity(header_len + n);
            frame.extend_from_slice(frame_header);
            frame.extend_from_slice(chunk);

            if let Err(e) = self.framed.send(Bytes::from(frame)).await {
                let err = ClientError::from(e);
                self.invoke_error_hook(&err).await;
                return Err(err);
            }
            frames_sent += 1;
        }

        Ok(SendStreamingOutcome { frames_sent })
    }
}

/// Result of a single chunk read from the body source.
enum ReadChunk {
    /// End of input — no more data to send.
    Eof,
    /// Successfully read `n` bytes into the buffer.
    Bytes(usize),
    /// An I/O error occurred during the read.
    Err(ClientError),
}

/// Read the next chunk from `reader` into `buf`, classifying the outcome.
async fn read_chunk<R: AsyncRead + Unpin>(reader: &mut R, buf: &mut [u8]) -> ReadChunk {
    match reader.read(buf).await {
        Ok(0) => ReadChunk::Eof,
        Ok(n) => ReadChunk::Bytes(n),
        Err(e) => ReadChunk::Err(ClientError::from(e)),
    }
}

/// Compute the effective chunk size for outbound streaming.
///
/// Returns an error if the header alone fills (or exceeds) the maximum
/// frame length, leaving no room for body data, or if the resolved chunk
/// size is zero.
fn effective_chunk_size(
    header_len: usize,
    max_frame_length: usize,
    config: &SendStreamingConfig,
) -> Result<usize, ClientError> {
    if header_len >= max_frame_length {
        return Err(ClientError::from(io::Error::new(
            io::ErrorKind::InvalidInput,
            concat!(
                "frame header length meets or exceeds max frame length; ",
                "no room for body data",
            ),
        )));
    }

    let available = max_frame_length - header_len;

    let size = match config.chunk_size {
        Some(requested) => requested.min(available),
        None => available,
    };

    if size == 0 {
        return Err(ClientError::from(io::Error::new(
            io::ErrorKind::InvalidInput,
            "chunk size must be greater than zero",
        )));
    }

    Ok(size)
}
