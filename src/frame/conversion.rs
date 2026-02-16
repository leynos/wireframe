//! Conversion helpers for length prefix encoding.
use std::io;

use super::format::Endianness;
use crate::byte_order::{
    read_network_u64,
    write_network_u16,
    write_network_u32,
    write_network_u64,
};

pub(crate) const ERR_UNSUPPORTED_PREFIX: &str = "unsupported length prefix size";
pub(crate) const ERR_FRAME_TOO_LARGE: &str = "frame too large";
pub(crate) const ERR_INCOMPLETE_PREFIX: &str = "incomplete length prefix";

#[inline]
fn u64_from_le_bytes(bytes: [u8; 8]) -> u64 {
    #[expect(
        clippy::little_endian_bytes,
        reason = "Wire endianness is explicit; from_le_bytes keeps decoding host-independent."
    )]
    u64::from_le_bytes(bytes)
}

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
    // NOTE: size is validated above; this is a defensive fallback.
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

    // Wire prefix declares its endianness; normalising into an 8-byte buffer and
    // using explicit conversion helpers keeps decoding deterministic on any host
    // CPU.
    let val = match endianness {
        Endianness::Big => read_network_u64(buf),
        Endianness::Little => u64_from_le_bytes(buf),
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
///
/// For big-endian, the function delegates to the typed network-order write
/// helpers mirroring how the read path delegates to `read_network_u64`.
fn write_bytes_with_endianness(value: u64, endianness: Endianness, prefix: &mut [u8]) {
    let size = prefix.len();
    match endianness {
        Endianness::Big => write_big_endian_prefix(value, size, prefix),
        Endianness::Little => write_little_endian_prefix(value, prefix),
    }
}

/// Encode `value` into `prefix` in network (big-endian) byte order.
///
/// Delegates to the typed `write_network_*` helpers so the encode and
/// decode paths share the same lint-suppressed conversion point.
/// Callers guarantee the value fits in the requested prefix size via
/// `checked_prefix_cast` in `convert_len_to_value`.
fn write_big_endian_prefix(value: u64, size: usize, prefix: &mut [u8]) {
    #[expect(
        clippy::cast_possible_truncation,
        reason = "caller validates value fits in prefix via checked_prefix_cast"
    )]
    match size {
        1 => prefix.copy_from_slice(&[value as u8]),
        2 => prefix.copy_from_slice(&write_network_u16(value as u16)),
        4 => prefix.copy_from_slice(&write_network_u32(value as u32)),
        8 => prefix.copy_from_slice(&write_network_u64(value)),
        _ => debug_assert!(false, "size validated upstream to be 1, 2, 4, or 8"),
    }
}

/// Encode `value` into `prefix` in little-endian byte order.
fn write_little_endian_prefix(value: u64, prefix: &mut [u8]) {
    for (i, byte) in prefix.iter_mut().enumerate() {
        let shift = 8 * i;
        *byte = ((value >> shift) & 0xff) as u8;
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

    #[expect(
        clippy::indexing_slicing,
        reason = "size validated to be within the 8-byte prefix buffer"
    )]
    let prefix = &mut out[..size];

    write_bytes_with_endianness(value, endianness, prefix);

    if let Some(tail) = out.get_mut(size..) {
        tail.fill(0);
    }

    Ok(size)
}
