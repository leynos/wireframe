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
//! See the [`recovery`] module for customizing error handling behaviour.

use std::io;

use bytes::{Bytes, BytesMut};
use tokio_util::codec::{Decoder, Encoder, LengthDelimitedCodec, LengthDelimitedCodecError};

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

/// Clamp a requested frame length to the supported range.
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
    /// Maximum payload accepted before the encoder reports a framing error.
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
    pub const fn max_frame_length(&self) -> usize { self.max_frame_length }

    /// Build a fresh tokio codec so encoder and decoder state remain isolated.
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

/// Decoder half of [`LengthDelimitedFrameCodec`].
///
/// The decoder is deliberately peek-based: an incomplete frame stays in the
/// read buffer untouched, so the connection loop can report the exact bytes
/// that were received when the peer disconnects. Only a buffer holding a
/// complete frame, or one whose declared length exceeds the configured
/// maximum, is handed to the inner decoder.
///
/// A truncated frame is reported as [`CodecError::Eof`] carrying
/// [`EofError::MidHeader`] or [`EofError::MidFrame`], which is the signal a
/// connection uses to run the protocol `on_eof` hook.
#[doc(hidden)]
pub struct LengthDelimitedDecoder {
    /// Stateful length-delimited decoder used for one stream.
    inner: LengthDelimitedCodec,
    /// Maximum payload length, mirrored here to classify a buffer without
    /// decoding it.
    max_frame_length: usize,
}

impl Decoder for LengthDelimitedDecoder {
    type Item = Bytes;
    type Error = io::Error;

    fn decode(&mut self, src: &mut BytesMut) -> Result<Option<Self::Item>, Self::Error> {
        match BufferState::classify(src, self.max_frame_length) {
            BufferState::HeaderIncomplete { bytes_received } => {
                src.reserve(LENGTH_HEADER_SIZE - bytes_received);
                Ok(None)
            }
            BufferState::PayloadIncomplete {
                bytes_received,
                expected,
            } => {
                src.reserve(expected - bytes_received);
                Ok(None)
            }
            // A complete frame is available, or the declared length is too
            // large. Delegating the oversized case preserves the framing error
            // and its `Drop` recovery policy instead of reporting truncation.
            BufferState::Complete | BufferState::Oversized => self
                .inner
                .decode(src)
                .map(|opt| opt.map(BytesMut::freeze))
                .map_err(|error| as_framing_error(error, src, self.max_frame_length)),
        }
    }

    fn decode_eof(&mut self, src: &mut BytesMut) -> Result<Option<Self::Item>, Self::Error> {
        match BufferState::classify(src, self.max_frame_length) {
            // Clean close: no data remains at a frame boundary.
            BufferState::HeaderIncomplete { bytes_received: 0 } => Ok(None),
            BufferState::HeaderIncomplete { bytes_received } => {
                Err(CodecError::Eof(EofError::MidHeader {
                    bytes_received,
                    header_size: LENGTH_HEADER_SIZE,
                })
                .into())
            }
            BufferState::PayloadIncomplete {
                bytes_received,
                expected,
            } => Err(CodecError::Eof(EofError::MidFrame {
                bytes_received,
                expected,
            })
            .into()),
            // Only a complete frame is decoded here, so the inner decoder
            // never has to guess why its input ended. An oversized frame is
            // still delegated, so its framing error survives to the caller.
            BufferState::Complete | BufferState::Oversized => self
                .inner
                .decode_eof(src)
                .map(|opt| opt.map(BytesMut::freeze))
                .map_err(|error| as_framing_error(error, src, self.max_frame_length)),
        }
    }
}

/// How much of the next frame the read buffer holds.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum BufferState {
    /// Fewer than [`LENGTH_HEADER_SIZE`] bytes are buffered.
    HeaderIncomplete {
        /// Header bytes received so far.
        bytes_received: usize,
    },
    /// The header is complete but the declared payload is not.
    PayloadIncomplete {
        /// Payload bytes received so far.
        bytes_received: usize,
        /// Payload length the header declares.
        expected: usize,
    },
    /// A complete frame is buffered.
    Complete,
    /// The declared payload exceeds the configured maximum.
    Oversized,
}

impl BufferState {
    /// Classify `src` without consuming any of it.
    fn classify(src: &BytesMut, max_frame_length: usize) -> Self {
        if src.len() < LENGTH_HEADER_SIZE {
            return Self::HeaderIncomplete {
                bytes_received: src.len(),
            };
        }
        let Some(declared) = declared_payload_length(src) else {
            // The header slice is present, so this is unreachable in practice;
            // treat it as an incomplete header rather than guessing.
            return Self::HeaderIncomplete {
                bytes_received: src.len(),
            };
        };
        if declared > max_frame_length {
            return Self::Oversized;
        }
        let received = src.len() - LENGTH_HEADER_SIZE;
        if received < declared {
            return Self::PayloadIncomplete {
                bytes_received: received,
                expected: declared,
            };
        }
        Self::Complete
    }
}

/// Read the big-endian length prefix without consuming it.
fn declared_payload_length(src: &BytesMut) -> Option<usize> {
    let header = src.get(..LENGTH_HEADER_SIZE)?;
    let bytes = <[u8; LENGTH_HEADER_SIZE]>::try_from(header).ok()?;
    usize::try_from(read_network_u32(bytes)).ok()
}

/// Reclassify `tokio_util`'s oversize error as a structured framing error.
///
/// The inner codec reports a length prefix beyond `max` as an [`io::Error`] of
/// kind `InvalidData` whose source is [`LengthDelimitedCodecError`]. Reported
/// unchanged, the recovery coordinator would present it to a
/// [`RecoveryPolicyHook`] as [`CodecError::Io`] and record `error_type="io"`,
/// so an operator could not tell a framing failure apart from a transport
/// failure.
///
/// The declared length is still in `src` because the inner decoder returns
/// before it consumes the header. If it cannot be read, the error is passed
/// through untouched rather than reporting a size that was never observed.
fn as_framing_error(error: io::Error, src: &BytesMut, max: usize) -> io::Error {
    let is_oversize = error.kind() == io::ErrorKind::InvalidData
        && error
            .get_ref()
            .and_then(|inner| inner.downcast_ref::<LengthDelimitedCodecError>())
            .is_some();
    if !is_oversize {
        return error;
    }

    declared_payload_length(src).map_or(error, |size| {
        CodecError::Framing(FramingError::OversizedFrame { size, max }).into()
    })
}

/// Recover the structured EOF condition from a decoder's I/O error.
///
/// The default [`LengthDelimitedDecoder`] reports truncation as
/// [`CodecError::Eof`]. A custom decoder opts in to `on_eof` dispatch by
/// returning the same error from its `decode_eof` implementation.
pub(crate) fn eof_condition(error: &io::Error) -> Option<EofError> {
    error
        .get_ref()
        .and_then(|inner| inner.downcast_ref::<CodecError>())
        .and_then(|codec_error| match codec_error {
            CodecError::Eof(eof) => Some(*eof),
            _ => None,
        })
}

/// Encoder half of [`LengthDelimitedFrameCodec`].
#[doc(hidden)]
pub struct LengthDelimitedEncoder {
    /// Stateful length-delimited encoder used for one stream.
    inner: LengthDelimitedCodec,
    /// Local limit checked before delegating to the underlying encoder.
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
        // `tokio_util` copies the payload into `dst`; the counter records that
        // copy so a byte-level baseline can report it even though the
        // allocation count does not change.
        crate::benchmark_counters::record_framed(item.len());
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
            max_frame_length: self.max_frame_length,
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
