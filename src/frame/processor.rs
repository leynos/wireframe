//! Frame processor implementations.

use bytes::{Buf, BytesMut};

use super::{conversion::ERR_FRAME_TOO_LARGE, format::LengthFormat};

/// Trait defining how raw bytes are decoded into frames and encoded back.
pub trait FrameProcessor: Send + Sync {
    /// Logical frame type extracted from the stream.
    type Frame;

    /// Error type returned by `decode` and `encode`.
    type Error: std::error::Error + Send + Sync + 'static;

    /// Attempt to decode the next frame from `src`.
    ///
    /// # Errors
    /// Returns an error if the bytes in `src` cannot be parsed into a complete frame.
    fn decode(&self, src: &mut BytesMut) -> Result<Option<Self::Frame>, Self::Error>;

    /// Encode `frame` and append the bytes to `dst`.
    ///
    /// # Errors
    /// Returns an error if the frame cannot be written to `dst`.
    fn encode(&self, frame: &Self::Frame, dst: &mut BytesMut) -> Result<(), Self::Error>;
}

/// Trait for parsing frame metadata from a header without decoding the payload.
pub trait FrameMetadata {
    /// Fully deserialised frame type.
    type Frame;

    /// Error produced when parsing the metadata.
    type Error: std::error::Error + Send + Sync + 'static;

    /// Parse frame metadata from `src`, returning the frame and bytes consumed.
    ///
    /// # Errors
    /// Returns an error if the bytes cannot be interpreted as valid metadata.
    fn parse(&self, src: &[u8]) -> Result<(Self::Frame, usize), Self::Error>;
}

/// Simple length-prefixed framing using a configurable length prefix.
#[derive(Clone, Copy, Debug)]
pub struct LengthPrefixedProcessor {
    format: LengthFormat,
}

impl LengthPrefixedProcessor {
    /// Creates a new `LengthPrefixedProcessor` with the specified length prefix format.
    #[must_use]
    pub const fn new(format: LengthFormat) -> Self { Self { format } }
}

impl Default for LengthPrefixedProcessor {
    fn default() -> Self { Self::new(LengthFormat::default()) }
}

impl FrameProcessor for LengthPrefixedProcessor {
    type Frame = Vec<u8>;
    type Error = std::io::Error;

    fn decode(&self, src: &mut BytesMut) -> Result<Option<Self::Frame>, Self::Error> {
        if src.len() < self.format.bytes {
            return Ok(None);
        }
        let len = self.format.read_len(&src[..self.format.bytes])?;
        let needed = self.format.bytes.checked_add(len).ok_or_else(|| {
            std::io::Error::new(std::io::ErrorKind::InvalidInput, ERR_FRAME_TOO_LARGE)
        })?;
        if src.len() < needed {
            return Ok(None);
        }
        src.advance(self.format.bytes);
        Ok(Some(src.split_to(len).freeze().to_vec()))
    }

    fn encode(&self, frame: &Self::Frame, dst: &mut BytesMut) -> Result<(), Self::Error> {
        dst.reserve(self.format.bytes + frame.len());
        self.format.write_len(frame.len(), dst)?;
        dst.extend_from_slice(frame);
        Ok(())
    }
}
