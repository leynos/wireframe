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
    // SAFETY: size is validated above; this is a defensive fallback.
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

/// Convert a length value into a u64 based on the prefix size.
///
/// Callers are expected to validate `size` against the supported set
/// `{1, 2, 4, 8}` before invoking this helper.
fn convert_len_to_value(len: usize, size: usize) -> io::Result<u64> {
    let value = match size {
        1 => u64::from(checked_prefix_cast::<u8>(len)?),
        2 => u64::from(checked_prefix_cast::<u16>(len)?),
        4 => u64::from(checked_prefix_cast::<u32>(len)?),
        8 => checked_prefix_cast(len)?,
        _ => {
            debug_assert!(false, "size should be validated upstream");
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                ERR_UNSUPPORTED_PREFIX,
            ));
        }
    };
    Ok(value)
}

/// Write a u64 into `prefix` according to the specified endianness.
fn write_bytes_with_endianness(value: u64, endianness: Endianness, prefix: &mut [u8]) {
    let size = prefix.len();
    match endianness {
        Endianness::Big => {
            for (i, byte) in prefix.iter_mut().enumerate() {
                let shift = 8 * (size - 1 - i);
                *byte = ((value >> shift) & 0xff) as u8;
            }
        }
        Endianness::Little => {
            for (i, byte) in prefix.iter_mut().enumerate() {
                let shift = 8 * i;
                *byte = ((value >> shift) & 0xff) as u8;
            }
        }
    }
}

/// Encodes an integer directly into `out` according to `size` and `endianness`.
///
/// The function supports prefix sizes of `1`, `2`, `4`, or `8` bytes.
///
/// # Errors
/// Returns [`io::ErrorKind::InvalidInput`] when the prefix size is unsupported
/// or when `len` does not fit into the requested prefix.
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

    let value = convert_len_to_value(len, size)?;

    let Some(prefix) = out.get_mut(..size) else {
        debug_assert!(false, "validated size should fit into prefix buffer");
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            ERR_UNSUPPORTED_PREFIX,
        ));
    };

    write_bytes_with_endianness(value, endianness, prefix);

    if let Some(tail) = out.get_mut(size..) {
        tail.fill(0);
    }

    Ok(size)
}
