//! Frame encoding and decoding traits.
//!
//! A `FrameProcessor` converts raw bytes into logical frames and back.
//! Implementations may use any framing strategy suitable for the
//! underlying transport.

use std::io;

use async_trait::async_trait;
use bytes::{Buf, BytesMut};

/// Trait defining how raw bytes are decoded into frames and how frames are
/// encoded back into bytes for transmission.
///
/// The `Frame` associated type represents a logical unit extracted from or
/// written to the wire. Errors are represented by the `Error` associated type,
/// which must implement [`std::error::Error`].
#[async_trait]
pub trait FrameProcessor: Send + Sync {
    /// Logical frame type extracted from the stream.
    type Frame;

    /// Error type returned by `decode` and `encode`.
    type Error: std::error::Error + Send + Sync + 'static;

    /// Attempt to decode the next frame from `src`.
    async fn decode(&mut self, src: &mut BytesMut) -> Result<Option<Self::Frame>, Self::Error>;

    /// Encode `frame` and append the bytes to `dst`.
    async fn encode(&mut self, frame: &Self::Frame, dst: &mut BytesMut) -> Result<(), Self::Error>;
}

/// Simple length-prefixed framing using big-endian u32 lengths.
pub struct LengthPrefixedProcessor;

#[async_trait]
impl FrameProcessor for LengthPrefixedProcessor {
    type Frame = Vec<u8>;
    type Error = std::io::Error;

    async fn decode(&mut self, src: &mut BytesMut) -> Result<Option<Self::Frame>, Self::Error> {
        if src.len() < 4 {
            return Ok(None);
        }
        let mut len_bytes = [0u8; 4];
        len_bytes.copy_from_slice(&src[..4]);
        let len = u32::from_be_bytes(len_bytes);
        let len_usize = usize::try_from(len).map_err(|_| io::Error::other("frame too large"))?;
        if src.len() < 4 + len_usize {
            return Ok(None);
        }
        src.advance(4);
        Ok(Some(src.split_to(len_usize).to_vec()))
    }

    async fn encode(&mut self, frame: &Self::Frame, dst: &mut BytesMut) -> Result<(), Self::Error> {
        use bytes::BufMut;
        dst.reserve(4 + frame.len());
        let len = u32::try_from(frame.len())
            .map_err(|_| io::Error::new(io::ErrorKind::InvalidInput, "frame too large"))?;
        dst.put_u32(len);
        dst.extend_from_slice(frame);
        Ok(())
    }
}
