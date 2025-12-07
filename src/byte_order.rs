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
        reason = "Network byte order is mandated; big-endian bytes keep the wire contract \
                  explicit."
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
        reason = "Network byte order is mandated; big-endian bytes keep the wire contract \
                  explicit."
    )]
    u16::from_be_bytes(bytes)
}
