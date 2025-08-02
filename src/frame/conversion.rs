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
///
/// # Panics
/// Panics if the bit-shifting within the `write_bytes` closure leaves bits of
/// `value` outside the `u8` range. This cannot occur for valid prefix sizes and
/// checked values.
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

    let write_bytes = |value: u64, e: Endianness, size: usize, out: &mut [u8]| match e {
        Endianness::Big => {
            for (i, b) in out.iter_mut().enumerate().take(size) {
                let shift = 8 * (size - 1 - i);
                *b = u8::try_from((value >> shift) & 0xff).expect("masked < 256");
            }
        }
        Endianness::Little => {
            for (i, b) in out.iter_mut().enumerate().take(size) {
                let shift = 8 * i;
                *b = u8::try_from((value >> shift) & 0xff).expect("masked < 256");
            }
        }
    };

    match size {
        1 => {
            let v: u8 = checked_prefix_cast(len)?;
            write_bytes(u64::from(v), endianness, 1, &mut out[..1]);
        }
        2 => {
            let v: u16 = checked_prefix_cast(len)?;
            write_bytes(u64::from(v), endianness, 2, &mut out[..2]);
        }
        4 => {
            let v: u32 = checked_prefix_cast(len)?;
            write_bytes(u64::from(v), endianness, 4, &mut out[..4]);
        }
        8 => {
            let v: u64 = checked_prefix_cast(len)?;
            write_bytes(v, endianness, 8, &mut out[..8]);
        }
        _ => unreachable!(),
    }

    out[size..].fill(0);

    Ok(size)
}
