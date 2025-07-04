//! Frame encoding and decoding traits.
//!
//! A `FrameProcessor` converts raw bytes into logical frames and back.
//! Implementations may use any framing strategy suitable for the
//! underlying transport.

use std::io;

use byteorder::{BigEndian, LittleEndian, ReadBytesExt};

const ERR_UNSUPPORTED_PREFIX: &str = "unsupported length prefix size";
const ERR_FRAME_TOO_LARGE: &str = "frame too large";
const ERR_INCOMPLETE_PREFIX: &str = "incomplete length prefix";

fn cast<T: TryFrom<usize>>(len: usize) -> io::Result<T> {
    T::try_from(len).map_err(|_| io::Error::new(io::ErrorKind::InvalidInput, ERR_FRAME_TOO_LARGE))
}

use bytes::{Buf, BufMut, BytesMut};

/// Converts a byte slice into a `u64` according to `size` and `endianness`.
///
/// Only prefix sizes of `1`, `2`, `4`, or `8` bytes are supported. `bytes` must
/// contain at least `size` bytes.
///
/// # Errors
/// Returns [`io::ErrorKind::InvalidInput`] if `size` is unsupported or
/// [`io::ErrorKind::UnexpectedEof`] if `bytes` is too short.
///
/// # Examples
///
/// ```rust,no_run,ignore
/// use crate::frame::{Endianness, bytes_to_u64};
/// let buf = [0x00, 0x10, 0x20, 0x30];
/// assert_eq!(bytes_to_u64(&buf, 2, Endianness::Big).unwrap(), 0x0010);
/// ```
pub(crate) fn bytes_to_u64(bytes: &[u8], size: usize, endianness: Endianness) -> io::Result<u64> {
    if !matches!(size, 1 | 2 | 4 | 8) {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            ERR_UNSUPPORTED_PREFIX,
        ));
    }
    if bytes.len() < size {
        return Err(io::Error::new(
            io::ErrorKind::UnexpectedEof,
            ERR_INCOMPLETE_PREFIX,
        ));
    }

    let mut cur = io::Cursor::new(&bytes[..size]);
    let val = match (size, endianness) {
        (1, _) => cur.read_u8().map(u64::from),
        (2, Endianness::Big) => cur.read_u16::<BigEndian>().map(u64::from),
        (2, Endianness::Little) => cur.read_u16::<LittleEndian>().map(u64::from),
        (4, Endianness::Big) => cur.read_u32::<BigEndian>().map(u64::from),
        (4, Endianness::Little) => cur.read_u32::<LittleEndian>().map(u64::from),
        (8, Endianness::Big) => cur.read_u64::<BigEndian>(),
        (8, Endianness::Little) => cur.read_u64::<LittleEndian>(),
        _ => unreachable!(),
    }?;
    Ok(val)
}

/// Encodes an integer directly into `out` according to `size` and `endianness`.
///
/// The function supports prefix sizes of `1`, `2`, `4`, or `8` bytes.
///
/// # Errors
/// Returns [`io::ErrorKind::InvalidInput`] if the size is unsupported or if
/// `len` does not fit into the prefix.
///
/// # Examples
///
/// ```rust,no_run,ignore
/// use crate::frame::{Endianness, u64_to_bytes};
/// let mut buf = [0u8; 8];
/// let written = u64_to_bytes(0x1234, 2, Endianness::Big, &mut buf).unwrap();
/// assert_eq!(&buf[..written], [0x12, 0x34]);
/// ```
#[must_use = "length prefix byte count must be used"]
pub(crate) fn u64_to_bytes(
    len: usize,
    size: usize,
    endianness: Endianness,
    out: &mut [u8; 8],
) -> io::Result<usize> {
    match (size, endianness) {
        (1, _) => {
            out[0] = cast::<u8>(len)?;
        }
        (2, Endianness::Big) => {
            out[..2].copy_from_slice(&cast::<u16>(len)?.to_be_bytes());
        }
        (2, Endianness::Little) => {
            out[..2].copy_from_slice(&cast::<u16>(len)?.to_le_bytes());
        }
        (4, Endianness::Big) => {
            out[..4].copy_from_slice(&cast::<u32>(len)?.to_be_bytes());
        }
        (4, Endianness::Little) => {
            out[..4].copy_from_slice(&cast::<u32>(len)?.to_le_bytes());
        }
        (8, Endianness::Big) => {
            out[..8].copy_from_slice(&cast::<u64>(len)?.to_be_bytes());
        }
        (8, Endianness::Little) => {
            out[..8].copy_from_slice(&cast::<u64>(len)?.to_le_bytes());
        }
        _ => {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                ERR_UNSUPPORTED_PREFIX,
            ));
        }
    }

    out[size..].fill(0);

    Ok(size)
}

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
    /// Creates a new `LengthFormat` with the specified number of bytes and
    /// endianness for the length prefix.
    ///
    /// # Parameters
    /// - `bytes`: The number of bytes used for the length prefix.
    /// - `endianness`: The byte order for encoding and decoding the length prefix.
    ///
    /// # Returns
    /// A `LengthFormat` configured with the given size and endianness.
    #[must_use]
    pub const fn new(bytes: usize, endianness: Endianness) -> Self { Self { bytes, endianness } }

    /// Creates a `LengthFormat` for a 2-byte big-endian length prefix.
    #[must_use]
    pub const fn u16_be() -> Self { Self::new(2, Endianness::Big) }

    /// Creates a `LengthFormat` for a 2-byte little-endian length prefix.
    #[must_use]
    pub const fn u16_le() -> Self { Self::new(2, Endianness::Little) }

    /// Creates a `LengthFormat` for a 4-byte big-endian length prefix.
    #[must_use]
    pub const fn u32_be() -> Self { Self::new(4, Endianness::Big) }

    /// Creates a `LengthFormat` for a 4-byte little-endian length prefix.
    #[must_use]
    pub const fn u32_le() -> Self { Self::new(4, Endianness::Little) }

    /// Reads a length prefix from a byte slice according to the configured prefix size and
    /// endianness.
    ///
    /// # Parameters
    /// - `bytes`: The byte slice containing the length prefix. Must be at least as long as the
    ///   configured prefix size.
    ///
    /// # Returns
    /// The decoded length as a `usize` if successful.
    ///
    /// # Errors
    /// Returns an error if the prefix size is unsupported or if the decoded length does not fit in
    /// a `usize`.
    fn read_len(&self, bytes: &[u8]) -> io::Result<usize> {
        let len = bytes_to_u64(bytes, self.bytes, self.endianness)?;
        usize::try_from(len)
            .map_err(|_| io::Error::new(io::ErrorKind::InvalidInput, ERR_FRAME_TOO_LARGE))
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
        let mut buf = [0u8; 8];
        let written = u64_to_bytes(len, self.bytes, self.endianness, &mut buf)?;
        dst.put_slice(&buf[..written]);
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

/// Trait for parsing frame metadata from a header without decoding the full payload.
///
/// Types implementing this trait can inspect the initial bytes of a frame to
/// determine routing information or other header fields. The associated
/// [`Frame`](FrameMetadata::Frame) represents the fully deserialised frame type
/// returned by [`parse`](FrameMetadata::parse).
pub trait FrameMetadata {
    /// Fully deserialised frame type.
    type Frame;

    /// Error produced when parsing the metadata.
    type Error: std::error::Error + Send + Sync + 'static;

    /// Parse frame metadata from `src`, returning the frame and bytes consumed.
    ///
    /// # Errors
    ///
    /// Returns an error if the bytes cannot be interpreted as a valid frame
    /// header for the implementing protocol.
    fn parse(&self, src: &[u8]) -> Result<(Self::Frame, usize), Self::Error>;
}

/// Simple length-prefixed framing using a configurable length prefix.
#[derive(Clone, Copy, Debug)]
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

#[cfg(test)]
mod tests {
    use rstest::rstest;

    use super::*;

    #[rstest]
    #[case(vec![0x12], 1, Endianness::Big, 0x12)]
    #[case(vec![0x12, 0x34], 2, Endianness::Big, 0x1234)]
    #[case(vec![0x34, 0x12], 2, Endianness::Little, 0x1234)]
    #[case(vec![0, 0, 0, 1], 4, Endianness::Big, 1)]
    #[case(vec![1, 0, 0, 0], 4, Endianness::Little, 1)]
    #[case(vec![0, 0, 0, 0, 0, 0, 0, 1], 8, Endianness::Big, 1)]
    #[case(vec![1, 0, 0, 0, 0, 0, 0, 0], 8, Endianness::Little, 1)]
    fn bytes_to_u64_ok(
        #[case] bytes: Vec<u8>,
        #[case] size: usize,
        #[case] endianness: Endianness,
        #[case] expected: u64,
    ) {
        assert_eq!(
            bytes_to_u64(&bytes, size, endianness).expect("bytes_to_u64 should succeed"),
            expected
        );
    }

    #[rstest]
    #[case(0x12usize, 1, Endianness::Big, vec![0x12])]
    #[case(0x1234usize, 2, Endianness::Big, vec![0x12, 0x34])]
    #[case(0x1234usize, 2, Endianness::Little, vec![0x34, 0x12])]
    #[case(1usize, 4, Endianness::Big, vec![0, 0, 0, 1])]
    #[case(1usize, 4, Endianness::Little, vec![1, 0, 0, 0])]
    #[case(1usize, 8, Endianness::Big, vec![0, 0, 0, 0, 0, 0, 0, 1])]
    #[case(1usize, 8, Endianness::Little, vec![1, 0, 0, 0, 0, 0, 0, 0])]
    fn u64_to_bytes_ok(
        #[case] value: usize,
        #[case] size: usize,
        #[case] endianness: Endianness,
        #[case] expected: Vec<u8>,
    ) {
        let mut buf = [0u8; 8];
        let written =
            u64_to_bytes(value, size, endianness, &mut buf).expect("u64_to_bytes should succeed");
        assert_eq!(written, size);
        assert_eq!(&buf[..written], expected.as_slice());
    }

    #[rstest]
    #[case(vec![0x01], 2, Endianness::Big)]
    #[case(vec![0x02, 0x03], 4, Endianness::Little)]
    fn bytes_to_u64_short(
        #[case] bytes: Vec<u8>,
        #[case] size: usize,
        #[case] endianness: Endianness,
    ) {
        let err = bytes_to_u64(&bytes, size, endianness)
            .expect_err("bytes_to_u64 must error for truncated prefix slice");
        assert_eq!(err.kind(), io::ErrorKind::UnexpectedEof);
    }

    #[rstest]
    #[case(vec![0x01, 0x02, 0x03], 3, Endianness::Big)]
    #[case(vec![0x01, 0x02, 0x03], 3, Endianness::Little)]
    fn bytes_to_u64_unsupported(
        #[case] bytes: Vec<u8>,
        #[case] size: usize,
        #[case] endianness: Endianness,
    ) {
        let err = bytes_to_u64(&bytes, size, endianness)
            .expect_err("bytes_to_u64 must fail for unsupported size");
        assert_eq!(err.kind(), io::ErrorKind::InvalidInput);
    }

    #[rstest]
    fn u64_to_bytes_large() {
        let mut buf = [0u8; 8];
        let err = u64_to_bytes(300, 1, Endianness::Big, &mut buf)
            .expect_err("u64_to_bytes must fail if value exceeds 1-byte prefix");
        assert_eq!(err.kind(), io::ErrorKind::InvalidInput);
    }

    #[rstest]
    #[case(1usize, 3, Endianness::Big)]
    #[case(1usize, 3, Endianness::Little)]
    fn u64_to_bytes_unsupported(
        #[case] value: usize,
        #[case] size: usize,
        #[case] endianness: Endianness,
    ) {
        let mut buf = [0u8; 8];
        let err = u64_to_bytes(value, size, endianness, &mut buf)
            .expect_err("u64_to_bytes must fail for unsupported size");
        assert_eq!(err.kind(), io::ErrorKind::InvalidInput);
    }
}
