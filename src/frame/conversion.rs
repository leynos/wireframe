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
    let prefix = bytes
        .get(..size)
        .ok_or_else(|| io::Error::new(io::ErrorKind::UnexpectedEof, ERR_INCOMPLETE_PREFIX))?;
    match endianness {
        Endianness::Big => {
            if let Some(dst) = buf.get_mut(8 - size..) {
                dst.copy_from_slice(prefix);
            } else {
                debug_assert!(false, "validated size should fit into prefix buffer");
                return Err(io::Error::new(
                    io::ErrorKind::InvalidInput,
                    ERR_UNSUPPORTED_PREFIX,
                ));
            }
        }
        Endianness::Little => {
            if let Some(dst) = buf.get_mut(..size) {
                dst.copy_from_slice(prefix);
            } else {
                debug_assert!(false, "validated size should fit into prefix buffer");
                return Err(io::Error::new(
                    io::ErrorKind::InvalidInput,
                    ERR_UNSUPPORTED_PREFIX,
                ));
            }
        }
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
            if let Some(prefix) = out.get_mut(..size) {
                for (i, b) in prefix.iter_mut().enumerate() {
                    let shift = 8 * (size - 1 - i);
                    *b = ((value >> shift) & 0xff) as u8;
                }
            } else {
                debug_assert!(false, "validated size should fit output buffer");
            }
        }
        Endianness::Little => {
            if let Some(prefix) = out.get_mut(..size) {
                for (i, b) in prefix.iter_mut().enumerate() {
                    let shift = 8 * i;
                    *b = ((value >> shift) & 0xff) as u8;
                }
            } else {
                debug_assert!(false, "validated size should fit output buffer");
            }
        }
    };

    match size {
        1 => {
            let v: u8 = checked_prefix_cast(len)?;
            write_bytes(u64::from(v), endianness, 1, out);
        }
        2 => {
            let v: u16 = checked_prefix_cast(len)?;
            write_bytes(u64::from(v), endianness, 2, out);
        }
        4 => {
            let v: u32 = checked_prefix_cast(len)?;
            write_bytes(u64::from(v), endianness, 4, out);
        }
        8 => {
            let v: u64 = checked_prefix_cast(len)?;
            write_bytes(v, endianness, 8, out);
        }
        _ => {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                ERR_UNSUPPORTED_PREFIX,
            ));
        }
    }

    if let Some(tail) = out.get_mut(size..) {
        tail.fill(0);
    }

    Ok(size)
}
