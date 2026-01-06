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
const LENGTH_HEADER_SIZE: usize = 4;

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
            return Err(CodecError::Eof(EofError::CleanClose).into());
        }

        // Try to decode any remaining data
        match self.inner.decode_eof(src) {
            Ok(Some(frame)) => Ok(Some(BytesMut::freeze(frame))),
            // Inner decoder returns Ok(None) or Err for incomplete data at EOF.
            // In both cases, convert to our structured EOF error.
            Ok(None) | Err(_) => Err(build_eof_error(src)),
        }
    }
}

/// Helper to build the appropriate EOF error based on remaining buffer state.
fn build_eof_error(src: &BytesMut) -> io::Error {
    use bytes::Buf;

    let bytes_received = src.len();
    if bytes_received < LENGTH_HEADER_SIZE {
        // EOF during header read
        return CodecError::Eof(EofError::MidHeader {
            bytes_received,
            header_size: LENGTH_HEADER_SIZE,
        })
        .into();
    }

    // EOF during payload read - we have a header but incomplete payload
    // Parse the length prefix to determine expected size
    // The length check above guarantees this won't be None
    let Some(mut header) = src.get(..LENGTH_HEADER_SIZE) else {
        return CodecError::Eof(EofError::MidHeader {
            bytes_received,
            header_size: LENGTH_HEADER_SIZE,
        })
        .into();
    };
    let expected = header.get_u32() as usize;
    CodecError::Eof(EofError::MidFrame {
        bytes_received: bytes_received.saturating_sub(LENGTH_HEADER_SIZE),
        expected,
    })
    .into()
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

    fn wrap_payload(&self, payload: Bytes) -> Self::Frame { payload }

    fn max_frame_length(&self) -> usize { self.max_frame_length }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn length_delimited_codec_clamps_max_frame_length() {
        let codec = LengthDelimitedFrameCodec::new(MAX_FRAME_LENGTH.saturating_add(1));
        assert_eq!(codec.max_frame_length(), MAX_FRAME_LENGTH);
    }

    #[test]
    fn length_delimited_codec_round_trips_payload() {
        let codec = LengthDelimitedFrameCodec::new(128);
        let mut encoder = codec.encoder();
        let mut decoder = codec.decoder();

        let payload = Bytes::from(vec![1_u8, 2, 3, 4]);
        let frame = codec.wrap_payload(payload.clone());

        let mut buf = BytesMut::new();
        encoder
            .encode(frame, &mut buf)
            .expect("encode should succeed");

        let decoded_frame = decoder
            .decode(&mut buf)
            .expect("decode should succeed")
            .expect("expected a frame");

        assert_eq!(
            LengthDelimitedFrameCodec::frame_payload(&decoded_frame),
            payload.as_ref()
        );
    }

    #[test]
    fn length_delimited_codec_rejects_oversized_payloads() {
        let codec = LengthDelimitedFrameCodec::new(MIN_FRAME_LENGTH);
        let mut encoder = codec.encoder();

        let payload = Bytes::from(vec![0_u8; MIN_FRAME_LENGTH.saturating_add(1)]);
        let frame = codec.wrap_payload(payload);
        let mut buf = BytesMut::new();

        let err = encoder
            .encode(frame, &mut buf)
            .expect_err("expected encode to fail for oversized frame");
        // The error is converted from CodecError::Framing to io::Error
        assert_eq!(err.kind(), io::ErrorKind::InvalidData);
    }

    #[test]
    fn length_delimited_wrap_payload_reuses_bytes() {
        let codec = LengthDelimitedFrameCodec::new(128);
        let payload = Bytes::from(vec![9_u8; 4]);
        let frame = codec.wrap_payload(payload.clone());

        assert_eq!(payload.len(), frame.len());
        assert_eq!(payload.as_ref().as_ptr(), frame.as_ref().as_ptr());
    }

    #[test]
    fn decode_eof_with_empty_buffer_returns_clean_close() {
        let codec = LengthDelimitedFrameCodec::new(128);
        let mut decoder = codec.decoder();
        let mut buf = BytesMut::new();

        let err = decoder
            .decode_eof(&mut buf)
            .expect_err("expected clean close error");
        assert_eq!(err.kind(), io::ErrorKind::UnexpectedEof);
        assert!(err.to_string().contains("connection closed"));
    }

    #[test]
    fn decode_eof_with_partial_header_returns_mid_header() {
        let codec = LengthDelimitedFrameCodec::new(128);
        let mut decoder = codec.decoder();
        // Only 2 bytes of the 4-byte header
        let mut buf = BytesMut::from(&[0x00, 0x10][..]);

        let err = decoder
            .decode_eof(&mut buf)
            .expect_err("expected mid-header error");
        assert_eq!(err.kind(), io::ErrorKind::UnexpectedEof);
        assert!(err.to_string().contains("header"));
    }

    #[test]
    fn decode_eof_with_partial_payload_returns_mid_frame() {
        let codec = LengthDelimitedFrameCodec::new(128);
        let mut dec = codec.decoder();
        // 4-byte header saying 16 bytes payload, but only 8 bytes of payload
        let mut buf = BytesMut::from(&[0x00, 0x00, 0x00, 0x10, 0x01, 0x02, 0x03, 0x04][..]);

        let err = dec
            .decode_eof(&mut buf)
            .expect_err("expected mid-frame error");
        assert_eq!(err.kind(), io::ErrorKind::UnexpectedEof);
        // bytes_received is payload bytes (4), expected is from header (16)
        assert!(err.to_string().contains('4') || err.to_string().contains("16"));
    }

    #[test]
    fn decode_eof_with_complete_frame_succeeds() {
        let codec = LengthDelimitedFrameCodec::new(128);
        let mut enc = codec.encoder();
        let mut dec = codec.decoder();

        let payload = Bytes::from(vec![1_u8, 2, 3, 4]);
        let frame = codec.wrap_payload(payload.clone());

        let mut buf = BytesMut::new();
        enc.encode(frame, &mut buf).expect("encode should succeed");

        // decode_eof should return the complete frame
        let result = dec
            .decode_eof(&mut buf)
            .expect("decode should succeed")
            .expect("expected a frame");
        assert_eq!(result.as_ref(), payload.as_ref());
    }
}
