//! Conversion helpers for length prefix encoding.
use std::io;

use super::format::Endianness;

pub(crate) const ERR_UNSUPPORTED_PREFIX: &str = "unsupported length prefix size";
pub(crate) const ERR_FRAME_TOO_LARGE: &str = "frame too large";
pub(crate) const ERR_INCOMPLETE_PREFIX: &str = "incomplete length prefix";

#[derive(Copy, Clone)]
enum PrefixErr {
    UnsupportedSize,
    Incomplete,
}

fn prefix_err(kind: PrefixErr) -> io::Error {
    match kind {
        PrefixErr::UnsupportedSize => {
            io::Error::new(io::ErrorKind::InvalidInput, ERR_UNSUPPORTED_PREFIX)
        }
        PrefixErr::Incomplete => {
            io::Error::new(io::ErrorKind::UnexpectedEof, ERR_INCOMPLETE_PREFIX)
        }
    }
}

/// Checked conversion from `usize` to a specific prefix integer type.
///
/// Returns `ERR_FRAME_TOO_LARGE` if the value does not fit in `T`.
fn checked_prefix_cast<T: TryFrom<usize>>(len: usize) -> io::Result<T> {
    T::try_from(len).map_err(|_| io::Error::new(io::ErrorKind::InvalidInput, ERR_FRAME_TOO_LARGE))
}

fn parse_bytes<const N: usize, F>(slice: &[u8], f: F) -> u64
where
    F: FnOnce([u8; N]) -> u64,
{
    f(slice[..N].try_into().expect("slice length checked"))
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
        return Err(prefix_err(PrefixErr::UnsupportedSize));
    }
    if bytes.len() < size {
        return Err(prefix_err(PrefixErr::Incomplete));
    }

    let val = match (size, endianness) {
        (1, _) => u64::from(bytes[0]),
        (2, Endianness::Big) => parse_bytes::<2, _>(bytes, |b| u16::from_be_bytes(b).into()),
        (2, Endianness::Little) => parse_bytes::<2, _>(bytes, |b| u16::from_le_bytes(b).into()),
        (4, Endianness::Big) => parse_bytes::<4, _>(bytes, |b| u32::from_be_bytes(b).into()),
        (4, Endianness::Little) => parse_bytes::<4, _>(bytes, |b| u32::from_le_bytes(b).into()),
        (8, Endianness::Big) => parse_bytes::<8, _>(bytes, u64::from_be_bytes),
        (8, Endianness::Little) => parse_bytes::<8, _>(bytes, u64::from_le_bytes),
        _ => unreachable!(),
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
    match (size, endianness) {
        (1, _) => {
            out[0] = checked_prefix_cast::<u8>(len)?;
        }
        (2, Endianness::Big) => {
            out[..2].copy_from_slice(&checked_prefix_cast::<u16>(len)?.to_be_bytes());
        }
        (2, Endianness::Little) => {
            out[..2].copy_from_slice(&checked_prefix_cast::<u16>(len)?.to_le_bytes());
        }
        (4, Endianness::Big) => {
            out[..4].copy_from_slice(&checked_prefix_cast::<u32>(len)?.to_be_bytes());
        }
        (4, Endianness::Little) => {
            out[..4].copy_from_slice(&checked_prefix_cast::<u32>(len)?.to_le_bytes());
        }
        (8, Endianness::Big) => {
            out[..8].copy_from_slice(&checked_prefix_cast::<u64>(len)?.to_be_bytes());
        }
        (8, Endianness::Little) => {
            out[..8].copy_from_slice(&checked_prefix_cast::<u64>(len)?.to_le_bytes());
        }
        _ => {
            return Err(prefix_err(PrefixErr::UnsupportedSize));
        }
    }

    out[size..].fill(0);

    Ok(size)
}
