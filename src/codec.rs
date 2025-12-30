//! Pluggable framing codecs for wire protocols.
//!
//! Codecs define how raw byte streams are split into frames and how outgoing
//! payloads are wrapped for transmission. The default implementation uses a
//! length-prefixed format compatible with the previous Wireframe behaviour.

use std::io;

use bytes::{Bytes, BytesMut};
use tokio_util::codec::{Decoder, Encoder, LengthDelimitedCodec};

pub(crate) const MIN_FRAME_LENGTH: usize = 64;
pub(crate) const MAX_FRAME_LENGTH: usize = 16 * 1024 * 1024;

pub(crate) fn clamp_frame_length(value: usize) -> usize {
    value.clamp(MIN_FRAME_LENGTH, MAX_FRAME_LENGTH)
}

/// Trait for pluggable frame codecs supporting different wire protocols.
///
/// Implementors define their own `Frame` type (for example, a struct carrying
/// transaction identifiers) and provide decoder/encoder instances.
pub trait FrameCodec: Send + Sync + 'static {
    /// Frame type produced by decoding.
    type Frame: Send + Sync + 'static;

    /// Create a Tokio decoder for this codec.
    fn decoder(&self) -> impl Decoder<Item = Self::Frame, Error = io::Error> + Send;

    /// Create a Tokio encoder for this codec.
    fn encoder(&self) -> impl Encoder<Self::Frame, Error = io::Error> + Send;

    /// Extract the message payload bytes from a frame.
    fn frame_payload(frame: &Self::Frame) -> &[u8];

    /// Wrap serialized payload bytes into a frame for sending.
    fn wrap_payload(payload: Vec<u8>) -> Self::Frame;

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
}

impl Default for LengthDelimitedFrameCodec {
    fn default() -> Self {
        Self {
            max_frame_length: 1024,
        }
    }
}

struct LengthDelimitedAdapter {
    inner: LengthDelimitedCodec,
}

impl LengthDelimitedAdapter {
    fn new(max_frame_length: usize) -> Self {
        Self {
            inner: LengthDelimitedCodec::builder()
                .max_frame_length(max_frame_length)
                .new_codec(),
        }
    }
}

impl Decoder for LengthDelimitedAdapter {
    type Item = Vec<u8>;
    type Error = io::Error;

    fn decode(&mut self, src: &mut BytesMut) -> Result<Option<Self::Item>, Self::Error> {
        self.inner
            .decode(src)
            .map(|opt| opt.map(|bytes| bytes.as_ref().to_vec()))
    }

    fn decode_eof(&mut self, src: &mut BytesMut) -> Result<Option<Self::Item>, Self::Error> {
        self.inner
            .decode_eof(src)
            .map(|opt| opt.map(|bytes| bytes.as_ref().to_vec()))
    }
}

impl Encoder<Vec<u8>> for LengthDelimitedAdapter {
    type Error = io::Error;

    fn encode(&mut self, item: Vec<u8>, dst: &mut BytesMut) -> Result<(), Self::Error> {
        self.inner.encode(Bytes::from(item), dst)
    }
}

impl FrameCodec for LengthDelimitedFrameCodec {
    type Frame = Vec<u8>;

    fn decoder(&self) -> impl Decoder<Item = Self::Frame, Error = io::Error> + Send {
        LengthDelimitedAdapter::new(self.max_frame_length)
    }

    fn encoder(&self) -> impl Encoder<Self::Frame, Error = io::Error> + Send {
        LengthDelimitedAdapter::new(self.max_frame_length)
    }

    fn frame_payload(frame: &Self::Frame) -> &[u8] { frame.as_slice() }

    fn wrap_payload(payload: Vec<u8>) -> Self::Frame { payload }

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

        let payload = vec![1_u8, 2, 3, 4];
        let frame = LengthDelimitedFrameCodec::wrap_payload(payload.clone());

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
            payload.as_slice()
        );
    }

    #[test]
    fn length_delimited_codec_rejects_oversized_payloads() {
        let codec = LengthDelimitedFrameCodec::new(MIN_FRAME_LENGTH);
        let mut encoder = codec.encoder();

        let payload = vec![0_u8; MIN_FRAME_LENGTH.saturating_add(1)];
        let frame = LengthDelimitedFrameCodec::wrap_payload(payload);
        let mut buf = BytesMut::new();

        let err = encoder
            .encode(frame, &mut buf)
            .expect_err("expected encode to fail for oversized frame");
        assert_eq!(err.kind(), io::ErrorKind::InvalidInput);
    }
}
