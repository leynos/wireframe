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
        if bytes.len() < self.bytes {
            return Err(io::Error::new(
                io::ErrorKind::UnexpectedEof,
                "length prefix truncated",
            ));
        }
        if !matches!(self.bytes, 1 | 2 | 4 | 8) {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "unsupported length prefix size",
            ));
        }

        let mut slice = &bytes[..self.bytes];
        let len = match self.endianness {
            Endianness::Big => slice.get_uint(self.bytes),
            Endianness::Little => slice.get_uint_le(self.bytes),
        if !matches!(self.bytes, 1 | 2 | 4 | 8) {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "unsupported length prefix size",
            ));
        }

        let len_u64 = u64::try_from(len)
            .map_err(|_| io::Error::new(io::ErrorKind::InvalidInput, "frame too large"))?;
        match self.endianness {
            Endianness::Big => dst.put_uint(len_u64, self.bytes),
            Endianness::Little => dst.put_uint_le(len_u64, self.bytes),
            }
        };
        usize::try_from(len).map_err(|_| io::Error::other("frame too large"))
    }

    /// Writes a length prefix to the destination buffer using the configured size and endianness.
    ///
    /// Returns an error if the length is too large to fit in the configured prefix size or if the
    /// prefix size is unsupported.
    ///
    /// # Parameters
    /// - `len`: The length value to encode and write.
    /// - `dst`: The buffer to which the encoded length prefix will be appended.
    ///
    /// # Errors
    /// Returns an error if `len` exceeds the maximum value for the configured prefix size or if the
    /// prefix size is not supported.
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
    /// Returns a `LengthFormat` using a 4-byte big-endian length prefix.
    ///
    /// This is the default format for length-prefixed framing.
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
    /// Creates a new `LengthPrefixedProcessor` with the specified length prefix
    /// format.
    ///
    /// # Parameters
    /// - `format`: The length prefix format to use for framing.
    ///
    /// # Returns
    /// A `LengthPrefixedProcessor` configured with the given length format.
    #[must_use]
    pub const fn new(format: LengthFormat) -> Self { Self { format } }
}

impl Default for LengthPrefixedProcessor {
    /// Creates a `LengthPrefixedProcessor` using the default length format (4-byte big-endian
    /// prefix).
    ///
    /// # Returns
    /// A processor configured for 4-byte big-endian length-prefixed framing.
    fn default() -> Self { Self::new(LengthFormat::default()) }
}

impl FrameProcessor for LengthPrefixedProcessor {
    type Frame = Vec<u8>;
    type Error = std::io::Error;

    /// Attempts to decode a single length-prefixed frame from the source buffer.
    ///
    /// Returns `Ok(Some(frame))` if a complete frame is available, `Ok(None)` if
    /// more data is needed, or an error if the length prefix is invalid or cannot
    /// be read according to the configured format.
    ///
    /// The source buffer is advanced past the decoded frame and its length prefix.
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

    /// Encodes a frame by prefixing it with its length and appending it to the destination buffer.
    ///
    /// The length prefix format is determined by the processor's configuration. Returns an error
    /// if the frame length cannot be represented in the configured format.
    fn encode(&self, frame: &Self::Frame, dst: &mut BytesMut) -> Result<(), Self::Error> {
        dst.reserve(self.format.bytes + frame.len());
        self.format.write_len(frame.len(), dst)?;
        dst.extend_from_slice(frame);
        Ok(())
    }
}
