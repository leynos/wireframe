//! Conversion helpers for length prefix encoding.
use std::io;

use super::format::Endianness;

pub(crate) const ERR_UNSUPPORTED_PREFIX: &str = "unsupported length prefix size";
pub(crate) const ERR_FRAME_TOO_LARGE: &str = "frame too large";
pub(crate) const ERR_INCOMPLETE_PREFIX: &str = "incomplete length prefix";

/// Checked conversion from `usize` to a specific prefix integer type.
///
/// Returns `ERR_FRAME_TOO_LARGE` if the value does not fit in `T`.
fn checked_prefix_cast<T: TryFrom<usize>>(len: usize) -> io::Result<T> {
    T::try_from(len).map_err(|_| io::Error::new(io::ErrorKind::InvalidInput, ERR_FRAME_TOO_LARGE))
}

/// Converts a byte slice into a `u64` according to `size` and `endianness`.
///
/// Only prefix sizes of `1`, `2`, `4`, or `8` bytes are supported. `bytes` must
/// contain at least `size` bytes.
///
/// # Errors
/// Returns [`io::ErrorKind::InvalidInput`] if `size` is unsupported or
/// [`io::ErrorKind::UnexpectedEof`] if `bytes` is too short.
pub fn bytes_to_u64(bytes: &[u8], size: usize, endianness: Endianness) -> io::Result<u64> {
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

    let mut buf = [0u8; 8];
    match endianness {
        Endianness::Big => buf[8 - size..].copy_from_slice(&bytes[..size]),
        Endianness::Little => buf[..size].copy_from_slice(&bytes[..size]),
    }

    let val = match endianness {
        Endianness::Big => u64::from_be_bytes(buf),
        Endianness::Little => u64::from_le_bytes(buf),
    };
    Ok(val)
}

/// Encodes an integer directly into `out` according to `size` and `endianness`.
///
/// The function supports prefix sizes of `1`, `2`, `4`, or `8` bytes.
///
/// # Errors
/// Returns [`io::ErrorKind::InvalidInput`] if the size is unsupported or if
/// `len` does not fit into the prefix.
#[must_use = "length prefix byte count must be used"]
pub fn u64_to_bytes(
    len: usize,
    size: usize,
    endianness: Endianness,
    out: &mut [u8; 8],
) -> io::Result<usize> {
    if !matches!(size, 1 | 2 | 4 | 8) {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            ERR_UNSUPPORTED_PREFIX,
        ));
    }

    let prefix_bytes = match (size, endianness) {
        (1, _) => checked_prefix_cast::<u8>(len)?.to_ne_bytes().to_vec(),
        (2, Endianness::Big) => checked_prefix_cast::<u16>(len)?.to_be_bytes().to_vec(),
        (2, Endianness::Little) => checked_prefix_cast::<u16>(len)?.to_le_bytes().to_vec(),
        (4, Endianness::Big) => checked_prefix_cast::<u32>(len)?.to_be_bytes().to_vec(),
        (4, Endianness::Little) => checked_prefix_cast::<u32>(len)?.to_le_bytes().to_vec(),
        (8, Endianness::Big) => checked_prefix_cast::<u64>(len)?.to_be_bytes().to_vec(),
        (8, Endianness::Little) => checked_prefix_cast::<u64>(len)?.to_le_bytes().to_vec(),
        _ => unreachable!(),
    };

    out[..size].copy_from_slice(&prefix_bytes);

    Ok(size)
}
