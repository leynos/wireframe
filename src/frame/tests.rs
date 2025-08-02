//! Unit tests for frame helpers.

use std::io;

use rstest::rstest;

use super::{conversion::*, format::*};

#[rstest]
#[case(vec![0x12], 1, Endianness::Big, 0x12)]
#[case(vec![0x12, 0x34], 2, Endianness::Big, 0x1234)]
#[case(vec![0x34, 0x12], 2, Endianness::Little, 0x1234)]
#[case(vec![0, 0, 0, 1], 4, Endianness::Big, 1)]
#[case(vec![1, 0, 0, 0], 4, Endianness::Little, 1)]
#[case(vec![0, 0, 0, 0, 0, 0, 0, 1], 8, Endianness::Big, 1)]
#[case(vec![1, 0, 0, 0, 0, 0, 0, 0], 8, Endianness::Little, 1)]
#[case(vec![0xFF], 1, Endianness::Big, 0xFF)]
#[case(vec![0xFF, 0xFF], 2, Endianness::Big, 0xFFFF)]
#[case(vec![0xFF, 0xFF, 0xFF, 0xFF], 4, Endianness::Big, 0xFFFF_FFFF)]
#[case(vec![0xFF; 8], 8, Endianness::Big, 0xFFFF_FFFF_FFFF_FFFF)]
fn bytes_to_u64_ok(
    #[case] bytes: Vec<u8>,
    #[case] size: usize,
    #[case] endianness: Endianness,
    #[case] expected: u64,
) {
    assert_eq!(
        bytes_to_u64(&bytes, size, endianness).expect("failed to convert"),
        expected
    );
}

#[rstest]
#[case(0x12usize, 1, Endianness::Big, vec![0x12])]
#[case(0x1234usize, 2, Endianness::Big, vec![0x12, 0x34])]
#[case(0x1234usize, 2, Endianness::Little, vec![0x34, 0x12])]
#[case(1usize, 4, Endianness::Big, vec![0, 0, 0, 1])]
#[case(1usize, 4, Endianness::Little, vec![1, 0, 0, 0])]
#[case(1usize, 8, Endianness::Big, vec![0, 0, 0, 0, 0, 0, 0, 1])]
#[case(1usize, 8, Endianness::Little, vec![1, 0, 0, 0, 0, 0, 0, 0])]
fn u64_to_bytes_ok(
    #[case] value: usize,
    #[case] size: usize,
    #[case] endianness: Endianness,
    #[case] expected: Vec<u8>,
) {
    let mut buf = [0u8; 8];
    let written = u64_to_bytes(value, size, endianness, &mut buf).expect("failed to encode u64");
    assert_eq!(written, size);
    assert_eq!(&buf[..written], expected.as_slice());
}

#[rstest]
#[case(vec![0x01], 2, Endianness::Big)]
#[case(vec![0x02, 0x03], 4, Endianness::Little)]
fn bytes_to_u64_short(#[case] bytes: Vec<u8>, #[case] size: usize, #[case] endianness: Endianness) {
    let err = bytes_to_u64(&bytes, size, endianness).unwrap_err();
    assert_eq!(err.kind(), io::ErrorKind::UnexpectedEof);
}

#[rstest]
#[case(vec![0x01, 0x02, 0x03], 3, Endianness::Big)]
#[case(vec![0x01, 0x02, 0x03], 3, Endianness::Little)]
fn bytes_to_u64_unsupported(
    #[case] bytes: Vec<u8>,
    #[case] size: usize,
    #[case] endianness: Endianness,
) {
    let err = bytes_to_u64(&bytes, size, endianness).unwrap_err();
    assert_eq!(err.kind(), io::ErrorKind::InvalidInput);
}

#[rstest]
fn u64_to_bytes_large() {
    let mut buf = [0u8; 8];
    let err = u64_to_bytes(300, 1, Endianness::Big, &mut buf).unwrap_err();
    assert_eq!(err.kind(), io::ErrorKind::InvalidInput);
}

#[test]
fn u64_to_bytes_zero_length() {
    let mut buf = [0u8; 8];
    let err = u64_to_bytes(0, 0, Endianness::Big, &mut buf)
        .expect_err("u64_to_bytes must fail if length is zero");
    assert_eq!(err.kind(), io::ErrorKind::InvalidInput);
}

#[rstest]
#[case(1usize, 3, Endianness::Big)]
#[case(1usize, 3, Endianness::Little)]
fn u64_to_bytes_unsupported(
    #[case] value: usize,
    #[case] size: usize,
    #[case] endianness: Endianness,
) {
    let mut buf = [0u8; 8];
    let err = u64_to_bytes(value, size, endianness, &mut buf).unwrap_err();
    assert_eq!(err.kind(), io::ErrorKind::InvalidInput);
}

#[rstest]
#[case(0x1234usize, 4, Endianness::Big)]
#[case(0x1234usize, 4, Endianness::Little)]
fn u64_to_bytes_zeroes_remainder(
    #[case] value: usize,
    #[case] size: usize,
    #[case] endianness: Endianness,
) {
    let mut buf = [0xaau8; 8];
    u64_to_bytes(value, size, endianness, &mut buf).unwrap();
    assert!(buf[size..].iter().all(|&b| b == 0));
}
