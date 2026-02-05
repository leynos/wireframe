//! Pluggable framing codecs for wire protocols.
//!
//! Codecs define how raw byte streams are split into frames and how outgoing
//! payloads are wrapped for transmission. The default implementation uses a
//! length-prefixed format compatible with the previous Wireframe behaviour.
//!
//! # Error Handling
//!
//! The codec layer provides a structured error taxonomy via [`CodecError`] that
//! distinguishes between framing errors, protocol errors, I/O errors, and EOF
//! conditions. See the [`error`] module for details.
//!
//! Recovery policies determine how errors are handled:
//!
//! - [`RecoveryPolicy::Drop`]: Discard the malformed frame and continue.
//! - [`RecoveryPolicy::Quarantine`]: Pause the connection temporarily.
//! - [`RecoveryPolicy::Disconnect`]: Terminate the connection.
//!
//! See the [`recovery`] module for customising error handling behaviour.

use std::io;

use bytes::{Bytes, BytesMut};
use tokio_util::codec::{Decoder, Encoder, LengthDelimitedCodec};

use crate::byte_order::read_network_u32;

pub mod error;
pub mod recovery;

pub use error::{CodecError, EofError, FramingError, ProtocolError};
pub use recovery::{
    CodecErrorContext,
    DefaultRecoveryPolicy,
    RecoveryConfig,
    RecoveryPolicy,
    RecoveryPolicyHook,
};

/// Minimum frame length in bytes.
///
/// Frame lengths passed to codec constructors are clamped to at least this
/// value to ensure enough space for protocol overhead.
pub const MIN_FRAME_LENGTH: usize = 64;

/// Maximum frame length in bytes (16 MiB).
///
/// Frame lengths passed to codec constructors are clamped to at most this
/// value to prevent unbounded memory allocation.
pub const MAX_FRAME_LENGTH: usize = 16 * 1024 * 1024;

pub(crate) fn clamp_frame_length(value: usize) -> usize {
    value.clamp(MIN_FRAME_LENGTH, MAX_FRAME_LENGTH)
}

#[doc(hidden)]
pub mod examples;

/// Trait for pluggable frame codecs supporting different wire protocols.
///
/// Implementors define their own `Frame` type (for example, a struct carrying
/// transaction identifiers) and provide decoder/encoder instances.
pub trait FrameCodec: Send + Sync + Clone + 'static {
    /// Frame type produced by decoding.
    type Frame: Send + Sync + 'static;
    /// Decoder type for this codec.
    type Decoder: Decoder<Item = Self::Frame, Error = io::Error> + Send;
    /// Encoder type for this codec.
    type Encoder: Encoder<Self::Frame, Error = io::Error> + Send;

    /// Create a Tokio decoder for this codec.
    fn decoder(&self) -> Self::Decoder;

    /// Create a Tokio encoder for this codec.
    fn encoder(&self) -> Self::Encoder;

    /// Extract the message payload bytes from a frame.
    fn frame_payload(frame: &Self::Frame) -> &[u8];

    /// Extract the message payload bytes from a frame as owned [`Bytes`].
    ///
    /// This method enables zero-copy payload extraction for codecs whose frame
    /// type uses `Bytes` internally. The default implementation copies the
    /// slice returned by [`frame_payload`][Self::frame_payload] into a new
    /// `Bytes` buffer.
    ///
    /// Override this method when the frame type can provide the payload
    /// without allocation.
    fn frame_payload_bytes(frame: &Self::Frame) -> Bytes {
        Bytes::copy_from_slice(Self::frame_payload(frame))
    }

    /// Wrap serialized payload bytes into a frame for sending.
    fn wrap_payload(&self, payload: Bytes) -> Self::Frame;

    /// Extract correlation ID for request/response matching.
    ///
    /// Returns `None` for protocols without correlation (for example, RESP).
    fn correlation_id(_frame: &Self::Frame) -> Option<u64> { None }

    /// Maximum frame length this codec will accept.
    fn max_frame_length(&self) -> usize;
}

/// Default codec using `tokio_util`'s `LengthDelimitedCodec`.
///
/// Provides backward compatibility with existing wireframe users. Uses a
/// 4-byte big-endian length prefix (`tokio_util` default).
#[derive(Clone, Debug)]
pub struct LengthDelimitedFrameCodec {
    max_frame_length: usize,
}

impl LengthDelimitedFrameCodec {
    /// Construct a new codec with a maximum frame length.
    #[must_use]
    pub fn new(max_frame_length: usize) -> Self {
        Self {
            max_frame_length: clamp_frame_length(max_frame_length),
        }
    }

    /// Return the maximum frame length accepted by this codec.
    #[must_use]
    pub fn max_frame_length(&self) -> usize { self.max_frame_length }

    fn new_inner_codec(&self) -> LengthDelimitedCodec {
        LengthDelimitedCodec::builder()
            .max_frame_length(self.max_frame_length)
            .new_codec()
    }
}

impl Default for LengthDelimitedFrameCodec {
    fn default() -> Self {
        Self {
            max_frame_length: 1024,
        }
    }
}

/// Length prefix header size (4 bytes for big-endian u32).
pub const LENGTH_HEADER_SIZE: usize = 4;

#[doc(hidden)]
pub struct LengthDelimitedDecoder {
    inner: LengthDelimitedCodec,
}

impl Decoder for LengthDelimitedDecoder {
    type Item = Bytes;
    type Error = io::Error;

    fn decode(&mut self, src: &mut BytesMut) -> Result<Option<Self::Item>, Self::Error> {
        self.inner.decode(src).map(|opt| opt.map(BytesMut::freeze))
    }

    fn decode_eof(&mut self, src: &mut BytesMut) -> Result<Option<Self::Item>, Self::Error> {
        // Clean close: no data remaining at frame boundary
        if src.is_empty() {
            return Ok(None);
        }

        // Try to decode any remaining data
        match self.inner.decode_eof(src) {
            Ok(Some(frame)) => Ok(Some(BytesMut::freeze(frame))),
            // Inner decoder returns Ok(None) for incomplete data at EOF - synthesise
            // our structured EOF error with context about what was received.
            Ok(None) => Err(build_eof_error(src)),
            // Inner decoder returned an error. Preserve framing errors (InvalidData)
            // like oversized frames to maintain correct recovery policy (Drop vs
            // Disconnect). For all other errors (typically incomplete data indicated
            // by Other kind), convert to our structured EOF error.
            Err(e) if e.kind() == io::ErrorKind::InvalidData => Err(e),
            Err(e) => {
                // Log inner error for diagnostics before replacing with structured EOF error
                tracing::debug!(
                    inner_error = %e,
                    "inner decoder error at EOF, converting to structured EOF error"
                );
                Err(build_eof_error(src))
            }
        }
    }
}

/// Build the appropriate EOF error based on remaining buffer state.
///
/// Determines whether the connection closed mid-header or mid-frame:
///
/// - [`EofError::MidHeader`]: Fewer than 4 bytes received (incomplete length prefix). The
///   connection closed before the full frame header could be read.
/// - [`EofError::MidFrame`]: Header complete but payload truncated. The length prefix was
///   successfully read but the connection closed before the full payload arrived.
fn build_eof_error(src: &BytesMut) -> io::Error {
    let bytes_received = src.len();

    // Use safe accessor to extract the length header if present
    let expected = src
        .get(..LENGTH_HEADER_SIZE)
        .and_then(|slice| <[u8; LENGTH_HEADER_SIZE]>::try_from(slice).ok())
        .map(|bytes| read_network_u32(bytes) as usize);

    match expected {
        Some(expected) => {
            // EOF during payload read - we have a header but incomplete payload
            CodecError::Eof(EofError::MidFrame {
                bytes_received: bytes_received.saturating_sub(LENGTH_HEADER_SIZE),
                expected,
            })
            .into()
        }
        None => {
            // EOF during header read
            CodecError::Eof(EofError::MidHeader {
                bytes_received,
                header_size: LENGTH_HEADER_SIZE,
            })
            .into()
        }
    }
}

#[doc(hidden)]
pub struct LengthDelimitedEncoder {
    inner: LengthDelimitedCodec,
    max_frame_length: usize,
}

impl Encoder<Bytes> for LengthDelimitedEncoder {
    type Error = io::Error;

    fn encode(&mut self, item: Bytes, dst: &mut BytesMut) -> Result<(), Self::Error> {
        if item.len() > self.max_frame_length {
            return Err(CodecError::Framing(FramingError::OversizedFrame {
                size: item.len(),
                max: self.max_frame_length,
            })
            .into());
        }
        self.inner.encode(item, dst)
    }
}

impl FrameCodec for LengthDelimitedFrameCodec {
    type Frame = Bytes;
    type Decoder = LengthDelimitedDecoder;
    type Encoder = LengthDelimitedEncoder;

    fn decoder(&self) -> Self::Decoder {
        LengthDelimitedDecoder {
            inner: self.new_inner_codec(),
        }
    }

    fn encoder(&self) -> Self::Encoder {
        LengthDelimitedEncoder {
            inner: self.new_inner_codec(),
            max_frame_length: self.max_frame_length,
        }
    }

    fn frame_payload(frame: &Self::Frame) -> &[u8] { frame.as_ref() }

    fn frame_payload_bytes(frame: &Self::Frame) -> Bytes { frame.clone() }

    fn wrap_payload(&self, payload: Bytes) -> Self::Frame { payload }

    fn max_frame_length(&self) -> usize { self.max_frame_length }
}

#[cfg(test)]
mod tests;
