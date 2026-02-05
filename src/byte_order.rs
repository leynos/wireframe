//! Helpers for explicit network byte-order conversions.
//!
//! These helpers keep Clippy expectations scoped to the conversion points so
//! protocol code can remain explicit about wire endianness without repeating
//! lint annotations.

/// Serialise a `u16` in network byte order (big-endian).
///
/// # Examples
///
/// ```
/// use wireframe::byte_order::write_network_u16;
///
/// assert_eq!(write_network_u16(0x1234), [0x12, 0x34]);
/// ```
#[must_use]
pub fn write_network_u16(value: u16) -> [u8; 2] {
    #[expect(
        clippy::big_endian_bytes,
        reason = "Network byte order requires big-endian bytes."
    )]
    value.to_be_bytes()
}

/// Parse a network-order `u16` from its on-wire representation.
///
/// # Examples
///
/// ```
/// use wireframe::byte_order::read_network_u16;
///
/// assert_eq!(read_network_u16([0x12, 0x34]), 0x1234);
/// ```
#[must_use]
pub fn read_network_u16(bytes: [u8; 2]) -> u16 {
    #[expect(
        clippy::big_endian_bytes,
        reason = "Network byte order requires big-endian bytes."
    )]
    u16::from_be_bytes(bytes)
}

/// Serialise a `u32` in network byte order (big-endian).
///
/// # Examples
///
/// ```
/// use wireframe::byte_order::write_network_u32;
///
/// assert_eq!(write_network_u32(0x1234_5678), [0x12, 0x34, 0x56, 0x78]);
/// ```
#[must_use]
pub fn write_network_u32(value: u32) -> [u8; 4] {
    #[expect(
        clippy::big_endian_bytes,
        reason = "Network byte order requires big-endian bytes."
    )]
    value.to_be_bytes()
}

/// Parse a network-order `u32` from its on-wire representation.
///
/// # Examples
///
/// ```
/// use wireframe::byte_order::read_network_u32;
///
/// assert_eq!(read_network_u32([0x12, 0x34, 0x56, 0x78]), 0x1234_5678);
/// ```
#[must_use]
pub fn read_network_u32(bytes: [u8; 4]) -> u32 {
    #[expect(
        clippy::big_endian_bytes,
        reason = "Network byte order requires big-endian bytes."
    )]
    u32::from_be_bytes(bytes)
}

/// Serialise a `u64` in network byte order (big-endian).
///
/// # Examples
///
/// ```
/// use wireframe::byte_order::write_network_u64;
///
/// assert_eq!(
///     write_network_u64(0x1122_3344_5566_7788),
///     [0x11, 0x22, 0x33, 0x44, 0x55, 0x66, 0x77, 0x88]
/// );
/// ```
#[must_use]
pub fn write_network_u64(value: u64) -> [u8; 8] {
    #[expect(
        clippy::big_endian_bytes,
        reason = "Network byte order requires big-endian bytes."
    )]
    value.to_be_bytes()
}

/// Parse a network-order `u64` from its on-wire representation.
///
/// # Examples
///
/// ```
/// use wireframe::byte_order::read_network_u64;
///
/// assert_eq!(
///     read_network_u64([0x11, 0x22, 0x33, 0x44, 0x55, 0x66, 0x77, 0x88]),
///     0x1122_3344_5566_7788
/// );
/// ```
#[must_use]
pub fn read_network_u64(bytes: [u8; 8]) -> u64 {
    #[expect(
        clippy::big_endian_bytes,
        reason = "Network byte order requires big-endian bytes."
    )]
    u64::from_be_bytes(bytes)
}

#[cfg(test)]
mod tests {
    use super::{
        read_network_u16,
        read_network_u32,
        read_network_u64,
        write_network_u16,
        write_network_u32,
        write_network_u64,
    };

    #[test]
    fn u16_round_trip() {
        let value = 0x1234;
        let bytes = write_network_u16(value);
        assert_eq!(bytes, [0x12, 0x34]);
        assert_eq!(read_network_u16(bytes), value);
    }

    #[test]
    fn u32_round_trip() {
        let value = 0x1234_5678;
        let bytes = write_network_u32(value);
        assert_eq!(bytes, [0x12, 0x34, 0x56, 0x78]);
        assert_eq!(read_network_u32(bytes), value);
    }

    #[test]
    fn u64_round_trip() {
        let value = 0x1122_3344_5566_7788;
        let bytes = write_network_u64(value);
        assert_eq!(bytes, [0x11, 0x22, 0x33, 0x44, 0x55, 0x66, 0x77, 0x88]);
        assert_eq!(read_network_u64(bytes), value);
    }
}
