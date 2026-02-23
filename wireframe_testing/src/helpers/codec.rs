//! Framing codec helpers used by test harness utilities.

use std::io;

use bytes::BytesMut;
use tokio_util::codec::{Decoder, Encoder, LengthDelimitedCodec};
use wireframe::frame::LengthFormat;

use super::TEST_MAX_FRAME;

/// Build a [`LengthDelimitedCodec`] configured with `max_len` as the maximum
/// accepted frame length.
///
/// The codec uses the default [`LengthFormat`] framing and enforces
/// `max_len` during decode.
#[inline]
pub fn new_test_codec(max_len: usize) -> LengthDelimitedCodec {
    let mut builder = LengthDelimitedCodec::builder();
    builder.max_frame_length(max_len);
    builder.new_codec()
}

/// Decode all length-prefixed `frames` using a test codec.
///
/// This helper constructs a [`LengthDelimitedCodec`] capped at [`TEST_MAX_FRAME`]
/// and decodes each frame in `bytes`, returning an error when decode fails or
/// trailing bytes remain.
///
/// ```rust
/// # use wireframe_testing::decode_frames;
/// let frames = decode_frames(vec![0, 0, 0, 1, 42])?;
/// assert_eq!(frames, vec![vec![42]]);
/// # Ok::<(), std::io::Error>(())
/// ```
pub fn decode_frames(bytes: Vec<u8>) -> io::Result<Vec<Vec<u8>>> {
    decode_frames_with_max(bytes, TEST_MAX_FRAME)
}

/// Decode `bytes` into frames using a codec capped at `max_len`.
///
/// Returns an error if decode fails or trailing bytes remain after frame
/// extraction.
pub fn decode_frames_with_max(bytes: Vec<u8>, max_len: usize) -> io::Result<Vec<Vec<u8>>> {
    let mut codec = new_test_codec(max_len);
    let mut buf = BytesMut::from(&bytes[..]);
    let mut frames = Vec::new();
    while let Some(frame) = codec.decode(&mut buf).map_err(|error| {
        io::Error::new(
            io::ErrorKind::InvalidData,
            format!("decode failed: {error}"),
        )
    })? {
        frames.push(frame.to_vec());
    }
    if !buf.is_empty() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            format!("unexpected trailing bytes after decode: {}", buf.len()),
        ));
    }
    Ok(frames)
}

/// Encode bytes with a length-delimited `codec`, preallocating the prefix.
///
/// Returns an error if framing fails.
pub fn encode_frame(codec: &mut LengthDelimitedCodec, bytes: Vec<u8>) -> io::Result<Vec<u8>> {
    let header_len = LengthFormat::default().bytes();
    let mut buf = BytesMut::with_capacity(bytes.len() + header_len);
    codec.encode(bytes.into(), &mut buf).map_err(|error| {
        io::Error::new(
            io::ErrorKind::InvalidData,
            format!("encode failed: {error}"),
        )
    })?;
    Ok(buf.to_vec())
}
