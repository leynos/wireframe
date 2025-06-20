//! Frame encoding and decoding traits.
//!
//! A `FrameProcessor` converts raw bytes into logical frames and back.
//! Implementations may use any framing strategy suitable for the
//! underlying transport.

use std::io;

use bytes::{Buf, BufMut, BytesMut};

/// Byte order used for encoding and decoding length prefixes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Endianness {
    /// Most significant byte first.
    Big,
    /// Least significant byte first.
    Little,
}

/// Format of the length prefix preceding each frame.
#[derive(Clone, Copy, Debug)]
pub struct LengthFormat {
    bytes: usize,
    endianness: Endianness,
}

impl LengthFormat {
    /// Create a new [`LengthFormat`].
    #[must_use]
    pub const fn new(bytes: usize, endianness: Endianness) -> Self { Self { bytes, endianness } }

    /// Two byte big-endian prefix.
    #[must_use]
    pub const fn u16_be() -> Self { Self::new(2, Endianness::Big) }

    /// Two byte little-endian prefix.
    #[must_use]
    pub const fn u16_le() -> Self { Self::new(2, Endianness::Little) }

    /// Four byte big-endian prefix.
    #[must_use]
    pub const fn u32_be() -> Self { Self::new(4, Endianness::Big) }

    /// Four byte little-endian prefix.
    #[must_use]
    pub const fn u32_le() -> Self { Self::new(4, Endianness::Little) }

    fn read_len(&self, bytes: &[u8]) -> io::Result<usize> {
        let len = match (self.bytes, self.endianness) {
            (1, _) => u64::from(u8::from_ne_bytes([bytes[0]])),
            (2, Endianness::Big) => u64::from(u16::from_be_bytes([bytes[0], bytes[1]])),
            (2, Endianness::Little) => u64::from(u16::from_le_bytes([bytes[0], bytes[1]])),
            (4, Endianness::Big) => {
                u64::from(u32::from_be_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]))
            }
            (4, Endianness::Little) => {
                u64::from(u32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]))
            }
            (8, Endianness::Big) => u64::from_be_bytes([
                bytes[0], bytes[1], bytes[2], bytes[3], bytes[4], bytes[5], bytes[6], bytes[7],
            ]),
            (8, Endianness::Little) => u64::from_le_bytes([
                bytes[0], bytes[1], bytes[2], bytes[3], bytes[4], bytes[5], bytes[6], bytes[7],
            ]),
            _ => {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidInput,
                    "unsupported length prefix size",
                ));
            }
        };
        usize::try_from(len).map_err(|_| io::Error::other("frame too large"))
    }

    fn write_len(&self, len: usize, dst: &mut BytesMut) -> io::Result<()> {
        match (self.bytes, self.endianness) {
            (1, _) => dst.put_u8(
                u8::try_from(len)
                    .map_err(|_| io::Error::new(io::ErrorKind::InvalidInput, "frame too large"))?,
            ),
            (2, Endianness::Big) => dst.put_slice(
                &u16::try_from(len)
                    .map_err(|_| io::Error::new(io::ErrorKind::InvalidInput, "frame too large"))?
                    .to_be_bytes(),
            ),
            (2, Endianness::Little) => dst.put_slice(
                &u16::try_from(len)
                    .map_err(|_| io::Error::new(io::ErrorKind::InvalidInput, "frame too large"))?
                    .to_le_bytes(),
            ),
            (4, Endianness::Big) => dst.put_slice(
                &u32::try_from(len)
                    .map_err(|_| io::Error::new(io::ErrorKind::InvalidInput, "frame too large"))?
                    .to_be_bytes(),
            ),
            (4, Endianness::Little) => dst.put_slice(
                &u32::try_from(len)
                    .map_err(|_| io::Error::new(io::ErrorKind::InvalidInput, "frame too large"))?
                    .to_le_bytes(),
            ),
            (8, Endianness::Big) => dst.put_slice(
                &u64::try_from(len)
                    .map_err(|_| io::Error::new(io::ErrorKind::InvalidInput, "frame too large"))?
                    .to_be_bytes(),
            ),
            (8, Endianness::Little) => dst.put_slice(
                &u64::try_from(len)
                    .map_err(|_| io::Error::new(io::ErrorKind::InvalidInput, "frame too large"))?
                    .to_le_bytes(),
            ),
            _ => {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidInput,
                    "unsupported length prefix size",
                ));
            }
        }
        Ok(())
    }
}

impl Default for LengthFormat {
    fn default() -> Self { Self::u32_be() }
}

/// Trait defining how raw bytes are decoded into frames and how frames are
/// encoded back into bytes for transmission.
///
/// The `Frame` associated type represents a logical unit extracted from or
/// written to the wire. Errors are represented by the `Error` associated type,
/// which must implement [`std::error::Error`].
/// Frame processors operate synchronously on in-memory buffers and need
/// no mutable state. The trait therefore uses `&self` receivers.
pub trait FrameProcessor: Send + Sync {
    /// Logical frame type extracted from the stream.
    type Frame;

    /// Error type returned by `decode` and `encode`.
    type Error: std::error::Error + Send + Sync + 'static;

    /// Attempt to decode the next frame from `src`.
    ///
    /// # Errors
    ///
    /// Returns an error if the bytes in `src` cannot be parsed into a complete frame.
    fn decode(&self, src: &mut BytesMut) -> Result<Option<Self::Frame>, Self::Error>;

    /// Encode `frame` and append the bytes to `dst`.
    ///
    /// # Errors
    ///
    /// Returns an error if the frame cannot be written to `dst`.
    fn encode(&self, frame: &Self::Frame, dst: &mut BytesMut) -> Result<(), Self::Error>;
}

/// Simple length-prefixed framing using a configurable length prefix.
pub struct LengthPrefixedProcessor {
    format: LengthFormat,
}

impl LengthPrefixedProcessor {
    /// Construct a processor with the provided [`LengthFormat`].
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
        if src.len() < self.format.bytes + len {
            return Ok(None);
        }
        src.advance(self.format.bytes);
        Ok(Some(src.split_to(len).to_vec()))
    }

    fn encode(&self, frame: &Self::Frame, dst: &mut BytesMut) -> Result<(), Self::Error> {
        dst.reserve(self.format.bytes + frame.len());
        self.format.write_len(frame.len(), dst)?;
        dst.extend_from_slice(frame);
        Ok(())
    }
}
