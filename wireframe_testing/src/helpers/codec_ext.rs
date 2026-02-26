//! Generic encode/decode helpers for any [`FrameCodec`] implementation.
//!
//! These functions translate between raw payload bytes and codec-framed wire
//! bytes, allowing test drivers to work transparently with arbitrary codecs.

use std::io;

use bytes::{Bytes, BytesMut};
use tokio_util::codec::{Decoder, Encoder};
use wireframe::codec::FrameCodec;

/// Encode each payload into wire bytes using `codec`.
///
/// For every payload the codec's [`FrameCodec::wrap_payload`] produces a
/// frame, which is then serialized through the codec's encoder. The resulting
/// raw byte vectors are suitable for writing directly to a duplex stream.
///
/// # Errors
///
/// Returns an error if the encoder rejects a frame (e.g. payload too large).
///
/// ```rust
/// use wireframe::codec::examples::HotlineFrameCodec;
/// use wireframe_testing::encode_payloads_with_codec;
///
/// let codec = HotlineFrameCodec::new(4096);
/// let frames = encode_payloads_with_codec(&codec, vec![vec![1, 2, 3]]).unwrap();
/// assert!(!frames.is_empty());
/// ```
pub fn encode_payloads_with_codec<F: FrameCodec>(
    codec: &F,
    payloads: Vec<Vec<u8>>,
) -> io::Result<Vec<Vec<u8>>> {
    let mut encoder = codec.encoder();
    payloads
        .into_iter()
        .map(|payload| {
            let frame = codec.wrap_payload(Bytes::from(payload));
            let mut buf = BytesMut::new();
            encoder.encode(frame, &mut buf).map_err(|error| {
                io::Error::new(
                    io::ErrorKind::InvalidData,
                    format!("codec encode failed: {error}"),
                )
            })?;
            Ok(buf.to_vec())
        })
        .collect()
}

/// Decode raw wire bytes into frames using `codec`.
///
/// The byte vector is fed into the codec's decoder until no more complete
/// frames can be extracted. Trailing bytes that do not form a complete frame
/// are silently ignored (matching the behaviour of a connection that is shut
/// down after the last response).
///
/// # Errors
///
/// Returns an error if the decoder encounters malformed data.
///
/// ```rust
/// use wireframe::codec::examples::HotlineFrameCodec;
/// use wireframe_testing::{decode_frames_with_codec, encode_payloads_with_codec};
///
/// let codec = HotlineFrameCodec::new(4096);
/// let wire: Vec<u8> = encode_payloads_with_codec(&codec, vec![vec![42]])
///     .unwrap()
///     .into_iter()
///     .flatten()
///     .collect();
/// let frames = decode_frames_with_codec(&codec, wire).unwrap();
/// assert_eq!(frames.len(), 1);
/// ```
pub fn decode_frames_with_codec<F: FrameCodec>(
    codec: &F,
    bytes: Vec<u8>,
) -> io::Result<Vec<F::Frame>> {
    let mut decoder = codec.decoder();
    let mut buf = BytesMut::from(bytes.as_slice());
    let mut frames = Vec::new();
    while let Some(frame) = decoder.decode(&mut buf).map_err(|error| {
        io::Error::new(
            io::ErrorKind::InvalidData,
            format!("codec decode failed: {error}"),
        )
    })? {
        frames.push(frame);
    }
    Ok(frames)
}

/// Extract raw payload bytes from a slice of codec frames.
///
/// Calls [`FrameCodec::frame_payload`] on each frame and collects the
/// results into owned byte vectors.
///
/// ```rust
/// use bytes::Bytes;
/// use wireframe::codec::{
///     FrameCodec,
///     examples::{HotlineFrame, HotlineFrameCodec},
/// };
/// use wireframe_testing::extract_payloads;
///
/// let frame = HotlineFrame {
///     transaction_id: 1,
///     payload: Bytes::from_static(b"hi"),
/// };
/// let payloads = extract_payloads::<HotlineFrameCodec>(&[frame]);
/// assert_eq!(payloads, vec![b"hi".to_vec()]);
/// ```
pub fn extract_payloads<F: FrameCodec>(frames: &[F::Frame]) -> Vec<Vec<u8>> {
    frames
        .iter()
        .map(|frame| F::frame_payload(frame).to_vec())
        .collect()
}
