//! Length prefix formatting options.
use std::io;

use bytes::BytesMut;

use super::conversion::{bytes_to_u64, u64_to_bytes};

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
    pub bytes: usize,
    pub endianness: Endianness,
}

impl LengthFormat {
    /// Creates a new `LengthFormat` with the specified number of bytes and
    /// endianness for the length prefix.
    ///
    /// # Panics
    ///
    /// Panics if `bytes` is not in `1..=8`.
    #[must_use]
    pub const fn new(bytes: usize, endianness: Endianness) -> Self {
        assert!(matches!(bytes, 1..=8), "invalid length-prefix width");
        Self { bytes, endianness }
    }

    /// Fallible constructor validating the prefix width.
    ///
    /// # Errors
    ///
    /// Returns an error if `bytes` is not in `1..=8`.
    pub fn try_new(bytes: usize, endianness: Endianness) -> io::Result<Self> {
        if !(1..=8).contains(&bytes) {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "invalid length-prefix width",
            ));
        }
        Ok(Self { bytes, endianness })
    }

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

    /// Read a length prefix from `bytes` according to this format.
    ///
    /// # Errors
    /// Returns an error if `bytes` are shorter than the prefix, if the
    /// configured prefix width is not in `1..=8`, or if the encoded length
    /// exceeds `usize`.
    pub fn read_len(&self, bytes: &[u8]) -> io::Result<usize> {
        let len = bytes_to_u64(bytes, self.bytes, self.endianness)?;
        usize::try_from(len).map_err(|_| {
            io::Error::new(
                io::ErrorKind::InvalidInput,
                super::conversion::ERR_FRAME_TOO_LARGE,
            )
        })
    }

    /// Write `len` to `dst` using this format's prefix encoding.
    ///
    /// # Errors
    /// Returns an error if `len` cannot be represented by the prefix size.
    pub fn write_len(&self, len: usize, dst: &mut BytesMut) -> io::Result<()> {
        let mut buf = [0u8; 8];
        let written = u64_to_bytes(len, self.bytes, self.endianness, &mut buf)?;
        dst.extend_from_slice(&buf[..written]);
        Ok(())
    }
}

impl Default for LengthFormat {
    fn default() -> Self { Self::u32_be() }
}
