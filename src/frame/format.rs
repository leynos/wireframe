//! Length prefix formatting options.
use std::io;

use bytes::BytesMut;

use super::conversion::{ERR_FRAME_TOO_LARGE, bytes_to_u64, u64_to_bytes};

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

    /// Returns the prefix width in bytes.
    #[must_use]
    pub const fn bytes(&self) -> usize { self.bytes }

    /// Returns the endianness used for the prefix.
    #[must_use]
    pub const fn endianness(&self) -> Endianness { self.endianness }

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
        let prefix = buf
            .get(..written)
            .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, ERR_FRAME_TOO_LARGE))?;
        dst.extend_from_slice(prefix);
        Ok(())
    }
}

impl Default for LengthFormat {
    fn default() -> Self { Self::u32_be() }
}

#[cfg(test)]
mod tests {
    use rstest::rstest;

    use super::*;

    #[rstest]
    #[case(1)]
    #[case(2)]
    #[case(3)]
    #[case(4)]
    #[case(5)]
    #[case(6)]
    #[case(7)]
    #[case(8)]
    fn new_accepts_valid_width(#[case] bytes: usize) {
        let fmt = LengthFormat::new(bytes, Endianness::Big);
        assert_eq!(fmt.bytes(), bytes);
        assert_eq!(fmt.endianness(), Endianness::Big);
    }

    #[rstest]
    #[case(0)]
    #[case(9)]
    fn new_panics_on_invalid_width(#[case] bytes: usize) {
        let res = std::panic::catch_unwind(|| LengthFormat::new(bytes, Endianness::Big));
        let err = res.expect_err("expected panic");
        let msg = err
            .downcast_ref::<&str>()
            .copied()
            .or_else(|| err.downcast_ref::<String>().map(String::as_str))
            .unwrap_or_default();
        assert!(
            msg.contains("invalid length-prefix width"),
            "unexpected panic message: {msg}"
        );
    }

    #[rstest]
    #[case(1)]
    #[case(8)]
    fn try_new_accepts_valid_width(#[case] bytes: usize) {
        let fmt =
            LengthFormat::try_new(bytes, Endianness::Little).expect("valid width must succeed");
        assert_eq!(fmt.bytes(), bytes);
        assert_eq!(fmt.endianness(), Endianness::Little);
    }

    #[rstest]
    #[case(0)]
    #[case(9)]
    fn try_new_rejects_invalid_width(#[case] bytes: usize) {
        let err =
            LengthFormat::try_new(bytes, Endianness::Big).expect_err("invalid width must error");
        assert_eq!(err.kind(), std::io::ErrorKind::InvalidInput);
        assert_eq!(err.to_string(), "invalid length-prefix width");
    }

    #[test]
    fn default_is_u32_be() {
        let d = LengthFormat::default();
        assert_eq!(d.bytes(), 4);
        assert_eq!(d.endianness(), Endianness::Big);
    }
}
