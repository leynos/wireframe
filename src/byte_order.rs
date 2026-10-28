//! Helpers for explicit network byte-order conversions.
//!
//! The fixed-size buffers match each integer width, so the `bytes` big-endian
//! operations cannot run out of capacity or input.

use bytes::{Buf, BufMut};

/// Serialize a `u16` in network byte order (big-endian).
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
    let mut wire_bytes = [0; 2];
    let mut destination = wire_bytes.as_mut_slice();
    destination.put_u16(value);
    wire_bytes
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
    let mut wire_bytes = bytes.as_slice();
    wire_bytes.get_u16()
}

/// Serialize a `u32` in network byte order (big-endian).
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
    let mut wire_bytes = [0; 4];
    let mut destination = wire_bytes.as_mut_slice();
    destination.put_u32(value);
    wire_bytes
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
    let mut wire_bytes = bytes.as_slice();
    wire_bytes.get_u32()
}

/// Serialize a `u64` in network byte order (big-endian).
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
    let mut wire_bytes = [0; 8];
    let mut destination = wire_bytes.as_mut_slice();
    destination.put_u64(value);
    wire_bytes
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
    let mut wire_bytes = bytes.as_slice();
    wire_bytes.get_u64()
}

#[cfg(test)]
mod tests {
    //! Round-trip tests for network byte-order conversion helpers.

    use proptest::prelude::{any, prop_assert_eq, proptest};
    use rstest::rstest;

    use super::{
        read_network_u16,
        read_network_u32,
        read_network_u64,
        write_network_u16,
        write_network_u32,
        write_network_u64,
    };

    /// Verify that each network-order write/read pair round-trips correctly.
    #[rstest]
    #[case::u16(
        0x1234u64,
        &write_network_u16(0x1234)[..],
        &[0x12, 0x34],
        u64::from(read_network_u16([0x12, 0x34]))
    )]
    #[case::u32(
        0x1234_5678u64,
        &write_network_u32(0x1234_5678)[..],
        &[0x12, 0x34, 0x56, 0x78],
        u64::from(read_network_u32([0x12, 0x34, 0x56, 0x78]))
    )]
    #[case::u64(
        0x1122_3344_5566_7788u64,
        &write_network_u64(0x1122_3344_5566_7788)[..],
        &[0x11, 0x22, 0x33, 0x44, 0x55, 0x66, 0x77, 0x88],
        read_network_u64([0x11, 0x22, 0x33, 0x44, 0x55, 0x66, 0x77, 0x88])
    )]
    fn network_byte_order_round_trip(
        #[case] value: u64,
        #[case] written: &[u8],
        #[case] expected_bytes: &[u8],
        #[case] read_back: u64,
    ) {
        assert_eq!(written, expected_bytes);
        assert_eq!(read_back, value);
    }

    /// Zero encodes to the same-width zero buffer at every supported width.
    #[test]
    fn network_byte_order_boundaries_encode_zero() {
        assert_eq!(write_network_u16(0), [0; 2]);
        assert_eq!(write_network_u32(0), [0; 4]);
        assert_eq!(write_network_u64(0), [0; 8]);
    }

    /// The maximum value encodes to and decodes from an all-ones buffer.
    #[test]
    fn network_byte_order_boundaries_round_trip_maximum() {
        assert_eq!(write_network_u16(u16::MAX), [0xff; 2]);
        assert_eq!(write_network_u32(u32::MAX), [0xff; 4]);
        assert_eq!(write_network_u64(u64::MAX), [0xff; 8]);
        assert_eq!(read_network_u16([0xff; 2]), u16::MAX);
        assert_eq!(read_network_u32([0xff; 4]), u32::MAX);
        assert_eq!(read_network_u64([0xff; 8]), u64::MAX);
    }

    proptest! {
        /// Encoding and decoding preserve every bit at each supported width.
        #[test]
        fn network_byte_order_round_trips_arbitrary_values(
            short in any::<u16>(),
            medium in any::<u32>(),
            long in any::<u64>(),
        ) {
            prop_assert_eq!(read_network_u16(write_network_u16(short)), short);
            prop_assert_eq!(read_network_u32(write_network_u32(medium)), medium);
            prop_assert_eq!(read_network_u64(write_network_u64(long)), long);
        }
    }
}
