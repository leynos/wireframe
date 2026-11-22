//! A frame codec whose wrapping step is observable in tests.
//!
//! The default length-delimited codec wraps a payload by returning it
//! unchanged, so a hook that ran before wrapping would see exactly the bytes a
//! hook that ran after wrapping sees. Tests that pin the dispatch order use
//! this codec instead: its wrapper appends a tag, which makes the two orders
//! produce different frames. It is an outbound fixture: use it to wrap frames
//! whose wire bytes a test asserts on, not to build input for a decoder, which
//! would see [`WRAP_TAG`] as message data. Include this module with
//! `#[path = "common/tagging_codec.rs"] mod tagging_codec;`.

use bytes::Bytes;
use wireframe::codec::{FrameCodec, LengthDelimitedFrameCodec};

/// Byte appended while wrapping a payload.
pub const WRAP_TAG: u8 = 0x7e;

/// Length-delimited codec that appends [`WRAP_TAG`] to every wrapped payload.
#[derive(Clone, Debug)]
pub struct TaggingCodec {
    inner: LengthDelimitedFrameCodec,
}

impl TaggingCodec {
    /// Build a codec whose limit applies to the wrapped frame.
    ///
    /// `max_frame_length` bounds the frame the encoder emits, so it includes
    /// the [`WRAP_TAG`] byte this codec appends: a payload of exactly
    /// `max_frame_length` bytes becomes an oversized frame the encoder
    /// rejects.
    pub fn new(max_frame_length: usize) -> Self {
        Self {
            inner: LengthDelimitedFrameCodec::new(max_frame_length),
        }
    }
}

impl FrameCodec for TaggingCodec {
    type Frame = Bytes;
    type Decoder = <LengthDelimitedFrameCodec as FrameCodec>::Decoder;
    type Encoder = <LengthDelimitedFrameCodec as FrameCodec>::Encoder;

    fn decoder(&self) -> Self::Decoder { self.inner.decoder() }

    fn encoder(&self) -> Self::Encoder { self.inner.encoder() }

    fn frame_payload(frame: &Self::Frame) -> &[u8] { frame.as_ref() }

    fn frame_payload_bytes(frame: &Self::Frame) -> Bytes { frame.clone() }

    fn wrap_payload(&self, payload: Bytes) -> Self::Frame {
        let mut bytes = payload.to_vec();
        bytes.push(WRAP_TAG);
        Bytes::from(bytes)
    }

    fn max_frame_length(&self) -> usize { self.inner.max_frame_length() }
}
