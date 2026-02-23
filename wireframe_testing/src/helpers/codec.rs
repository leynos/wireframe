//! Framing codec helpers used by test harness utilities.

use bytes::BytesMut;
use tokio_util::codec::{Decoder, Encoder, LengthDelimitedCodec};
use wireframe::frame::LengthFormat;

use super::TEST_MAX_FRAME;

#[inline]
pub fn new_test_codec(max_len: usize) -> LengthDelimitedCodec {
    let mut builder = LengthDelimitedCodec::builder();
    builder.max_frame_length(max_len);
    builder.new_codec()
}

/// Decode all length-prefixed `frames` using a test codec and assert no bytes remain.
///
/// This helper constructs a [`LengthDelimitedCodec`] capped at [`TEST_MAX_FRAME`]
/// and decodes each frame in `bytes`, ensuring the buffer is fully consumed.
///
/// ```rust
/// # use wireframe_testing::decode_frames;
/// let frames = decode_frames(vec![0, 0, 0, 1, 42]);
/// assert_eq!(frames, vec![vec![42]]);
/// ```
#[must_use]
pub fn decode_frames(bytes: Vec<u8>) -> Vec<Vec<u8>> {
    decode_frames_with_max(bytes, TEST_MAX_FRAME)
}

/// Decode `bytes` into frames using a codec capped at `max_len`.
///
/// Asserts that no trailing bytes remain after all frames are decoded.
#[must_use]
pub fn decode_frames_with_max(bytes: Vec<u8>, max_len: usize) -> Vec<Vec<u8>> {
    let mut codec = new_test_codec(max_len);
    let mut buf = BytesMut::from(&bytes[..]);
    let mut frames = Vec::new();
    while let Some(frame) = codec.decode(&mut buf).expect("decode failed") {
        frames.push(frame.to_vec());
    }
    assert!(buf.is_empty(), "unexpected trailing bytes after decode");
    frames
}

/// Encode bytes with a length-delimited `codec`, preallocating the prefix.
///
/// Panics if encoding fails.
#[must_use]
pub fn encode_frame(codec: &mut LengthDelimitedCodec, bytes: Vec<u8>) -> Vec<u8> {
    let header_len = LengthFormat::default().bytes();
    let mut buf = BytesMut::with_capacity(bytes.len() + header_len);
    codec.encode(bytes.into(), &mut buf).expect("encode failed");
    buf.to_vec()
}
