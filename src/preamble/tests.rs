//! Regressions for preamble allocation limits and the read-buffer growth guard.

use std::io::Cursor;

use rstest::rstest;

use super::{MAX_PREAMBLE_LEN, PREAMBLE_TOO_LONG_ERROR, read_more, read_preamble};

/// Configured preamble type containing an owned, peer-sized container.
#[derive(Debug, PartialEq, bincode::Encode, bincode::Decode)]
struct ContainerPreamble(Vec<u8>);

#[rstest]
#[case::above_preamble_ceiling(MAX_PREAMBLE_LEN as u64 + 1)]
#[case::maximum_native_length(usize::MAX as u64)]
#[tokio::test]
async fn oversized_preamble_container_fails_before_reading_its_body(#[case] declared: u64) {
    // Fixed-width big-endian encoding allows the entire attack input to fit
    // in eight bytes, without constructing the declared container.
    let prefix = crate::byte_order::write_network_u64(declared);
    let mut reader = Cursor::new(prefix);
    let error = read_preamble::<_, ContainerPreamble>(&mut reader)
        .await
        .expect_err("declared allocation must exceed the preamble budget");
    assert!(
        matches!(error, bincode::error::DecodeError::LimitExceeded),
        "{error:?}"
    );
    assert_eq!(reader.position(), 8);
    assert!(reader.position() <= MAX_PREAMBLE_LEN as u64);
}

#[tokio::test]
async fn preamble_read_buffer_never_grows_past_ceiling() {
    let mut buffer = vec![0; MAX_PREAMBLE_LEN];
    let mut reader = Cursor::new([1]);
    let error = read_more(&mut reader, &mut buffer, 1)
        .await
        .expect_err("growth past the preamble cap must fail");
    assert!(matches!(
        error,
        bincode::error::DecodeError::Other(PREAMBLE_TOO_LONG_ERROR)
    ));
    assert_eq!(buffer.len(), MAX_PREAMBLE_LEN);
    assert_eq!(reader.position(), 0);
}

#[rstest]
#[case::empty(0)]
#[case::at_accounted_limit(MAX_PREAMBLE_LEN - std::mem::size_of::<u64>())]
#[tokio::test]
async fn bounded_preamble_preserves_valid_containers_and_following_bytes(#[case] length: usize) {
    let value = ContainerPreamble(vec![7; length]);
    let mut bytes = bincode::encode_to_vec(
        &value,
        bincode::config::standard()
            .with_big_endian()
            .with_fixed_int_encoding(),
    )
    .expect("encode valid preamble");
    let encoded_len = bytes.len();
    bytes.extend_from_slice(&[9, 10]);
    let mut reader = Cursor::new(bytes);
    let (decoded, leftover) = read_preamble::<_, ContainerPreamble>(&mut reader)
        .await
        .expect("preamble within the budget must decode");
    assert_eq!(decoded, value);
    assert_eq!(leftover, [] as [u8; 0]);
    assert_eq!(reader.position(), encoded_len as u64);
    assert!(encoded_len <= MAX_PREAMBLE_LEN);
    assert_eq!(
        reader.get_ref().get(encoded_len..),
        Some([9, 10].as_slice())
    );
}

#[tokio::test]
async fn preamble_returns_read_ahead_bytes_as_leftovers() {
    let mut reader = Cursor::new([9, 10]);
    let ((), leftover) = read_preamble::<_, ()>(&mut reader)
        .await
        .expect("empty preamble must preserve bytes read ahead");
    assert_eq!(leftover, [9]);
    assert_eq!(reader.position(), 1);
}
