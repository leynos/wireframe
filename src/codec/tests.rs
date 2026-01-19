//! Unit tests for the length-delimited frame codec.
//!
//! Tests codec construction, frame round-tripping, oversized payload rejection,
//! and EOF handling behaviour.

use std::io;

use bytes::{Bytes, BytesMut};
use rstest::rstest;

use super::*;

#[test]
fn length_delimited_codec_clamps_max_frame_length() {
    let codec = LengthDelimitedFrameCodec::new(MAX_FRAME_LENGTH.saturating_add(1));
    assert_eq!(codec.max_frame_length(), MAX_FRAME_LENGTH);
}

#[test]
fn length_delimited_codec_round_trips_payload() {
    let codec = LengthDelimitedFrameCodec::new(128);
    let mut encoder = codec.encoder();
    let mut decoder = codec.decoder();

    let payload = Bytes::from(vec![1_u8, 2, 3, 4]);
    let frame = codec.wrap_payload(payload.clone());

    let mut buf = BytesMut::new();
    encoder
        .encode(frame, &mut buf)
        .expect("encode should succeed");

    let decoded_frame = decoder
        .decode(&mut buf)
        .expect("decode should succeed")
        .expect("expected a frame");

    assert_eq!(
        LengthDelimitedFrameCodec::frame_payload(&decoded_frame),
        payload.as_ref()
    );
}

#[test]
fn length_delimited_codec_rejects_oversized_payloads() {
    let codec = LengthDelimitedFrameCodec::new(MIN_FRAME_LENGTH);
    let mut encoder = codec.encoder();

    let payload = Bytes::from(vec![0_u8; MIN_FRAME_LENGTH.saturating_add(1)]);
    let frame = codec.wrap_payload(payload);
    let mut buf = BytesMut::new();

    let err = encoder
        .encode(frame, &mut buf)
        .expect_err("expected encode to fail for oversized frame");
    // The error is converted from CodecError::Framing to io::Error
    assert_eq!(err.kind(), io::ErrorKind::InvalidData);
}

#[test]
fn length_delimited_wrap_payload_reuses_bytes() {
    let codec = LengthDelimitedFrameCodec::new(128);
    let payload = Bytes::from(vec![9_u8; 4]);
    let frame = codec.wrap_payload(payload.clone());

    assert_eq!(payload.len(), frame.len());
    assert_eq!(payload.as_ref().as_ptr(), frame.as_ref().as_ptr());
}

#[test]
fn decode_eof_with_empty_buffer_returns_none() {
    // Empty buffer: connection closed cleanly at frame boundary - returns Ok(None)
    let codec = LengthDelimitedFrameCodec::new(128);
    let mut decoder = codec.decoder();
    let mut buf = BytesMut::new();

    let result = decoder.decode_eof(&mut buf);
    assert!(
        matches!(result, Ok(None)),
        "clean close should return Ok(None), got {result:?}"
    );
}

/// Parameterized EOF error tests for the length-delimited decoder.
///
/// Each case specifies:
/// - `initial_buffer`: bytes to seed the buffer with
/// - `expected_kind`: the expected `io::ErrorKind`
/// - `expected_substring`: a substring that must appear in the error message
#[rstest]
#[case::partial_header(&[0x00, 0x10], io::ErrorKind::UnexpectedEof, "header")]
#[case::partial_payload(&[0x00, 0x00, 0x00, 0x10, 0x01, 0x02, 0x03, 0x04], io::ErrorKind::UnexpectedEof, "16")]
fn decode_eof_error_cases(
    #[case] initial_buffer: &[u8],
    #[case] expected_kind: io::ErrorKind,
    #[case] expected_substring: &str,
) {
    let codec = LengthDelimitedFrameCodec::new(128);
    let mut decoder = codec.decoder();
    let mut buf = BytesMut::from(initial_buffer);

    let err = decoder.decode_eof(&mut buf).expect_err("expected error");
    assert_eq!(err.kind(), expected_kind, "unexpected error kind");
    assert!(
        err.to_string().contains(expected_substring),
        "error message should contain '{expected_substring}', got: {err}"
    );
}

#[test]
fn decode_eof_with_complete_frame_succeeds() {
    let codec = LengthDelimitedFrameCodec::new(128);
    let mut enc = codec.encoder();
    let mut dec = codec.decoder();

    let payload = Bytes::from(vec![1_u8, 2, 3, 4]);
    let frame = codec.wrap_payload(payload.clone());

    let mut buf = BytesMut::new();
    enc.encode(frame, &mut buf).expect("encode should succeed");

    // decode_eof should return the complete frame
    let result = dec
        .decode_eof(&mut buf)
        .expect("decode should succeed")
        .expect("expected a frame");
    assert_eq!(result.as_ref(), payload.as_ref());
}

// ---------------------------------------------------------------------------
// Zero-copy regression tests
// ---------------------------------------------------------------------------

#[test]
fn frame_payload_bytes_reuses_memory_for_length_delimited() {
    let codec = LengthDelimitedFrameCodec::new(128);
    let payload = Bytes::from(vec![1_u8, 2, 3, 4]);
    let frame = codec.wrap_payload(payload.clone());

    let extracted = LengthDelimitedFrameCodec::frame_payload_bytes(&frame);

    assert_eq!(
        frame.as_ref().as_ptr(),
        extracted.as_ref().as_ptr(),
        "frame_payload_bytes should return the same memory region"
    );
}

#[test]
fn hotline_decode_produces_zero_copy_payload() {
    use bytes::BufMut;
    use tokio_util::codec::Decoder;

    use super::examples::HotlineFrameCodec;

    // Build a valid Hotline frame in a buffer.
    // Header: data_size (u32) + total_size (u32) + transaction_id (u32) + reserved (8 bytes) = 20
    // bytes
    let payload_data: &[u8] = &[0xde, 0xad, 0xbe, 0xef];
    let data_size: u32 = 4;
    let total_size: u32 = 20 + 4;

    let mut buf = BytesMut::with_capacity(total_size as usize);
    // Hotline wire protocol uses big-endian encoding.
    buf.put_u32(data_size);
    buf.put_u32(total_size);
    buf.put_u32(42); // transaction_id
    buf.extend_from_slice(&[0_u8; 8]); // reserved
    buf.extend_from_slice(payload_data);

    // Record pointer to payload region in buffer before decode
    let payload_ptr = buf
        .get(20..)
        .expect("buffer should have at least 20 bytes")
        .as_ptr();

    let codec = HotlineFrameCodec::new(128);
    let mut decoder = codec.decoder();
    let frame = decoder
        .decode(&mut buf)
        .expect("decode should succeed")
        .expect("expected a frame");

    // The decoded payload should point to the same memory (zero-copy via freeze)
    assert_eq!(
        payload_ptr,
        frame.payload.as_ptr(),
        "decoded payload should reuse buffer memory (zero-copy)"
    );
}

#[test]
fn hotline_wrap_payload_reuses_bytes() {
    use super::examples::HotlineFrameCodec;

    let codec = HotlineFrameCodec::new(128);
    let payload = Bytes::from(vec![5_u8; 8]);
    let frame = codec.wrap_payload(payload.clone());

    assert_eq!(
        payload.as_ref().as_ptr(),
        frame.payload.as_ref().as_ptr(),
        "wrap_payload should reuse the Bytes without copying"
    );
}

#[test]
fn hotline_frame_payload_bytes_reuses_memory() {
    use super::examples::HotlineFrameCodec;

    let codec = HotlineFrameCodec::new(128);
    let payload = Bytes::from(vec![7_u8; 6]);
    let frame = codec.wrap_payload(payload.clone());

    let extracted = HotlineFrameCodec::frame_payload_bytes(&frame);

    assert_eq!(
        frame.payload.as_ptr(),
        extracted.as_ptr(),
        "frame_payload_bytes should return the same memory region"
    );
}

#[test]
fn mysql_decode_produces_zero_copy_payload() {
    use bytes::BufMut;
    use tokio_util::codec::Decoder;

    use super::examples::MysqlFrameCodec;

    // Build a valid MySQL frame in a buffer.
    // Header: 3-byte little-endian length + 1-byte sequence_id = 4 bytes
    let payload_data: &[u8] = &[0xca, 0xfe, 0xba, 0xbe];
    let payload_len: u32 = 4;

    let mut buf = BytesMut::with_capacity(8);
    // MySQL uses little-endian 3-byte length
    buf.put_u8((payload_len & 0xff) as u8);
    buf.put_u8(((payload_len >> 8) & 0xff) as u8);
    buf.put_u8(((payload_len >> 16) & 0xff) as u8);
    buf.put_u8(1); // sequence_id
    buf.extend_from_slice(payload_data);

    // Record pointer to payload region in buffer before decode
    let payload_ptr = buf
        .get(4..)
        .expect("buffer should have at least 4 bytes")
        .as_ptr();

    let codec = MysqlFrameCodec::new(128);
    let mut decoder = codec.decoder();
    let frame = decoder
        .decode(&mut buf)
        .expect("decode should succeed")
        .expect("expected a frame");

    // The decoded payload should point to the same memory (zero-copy via freeze)
    assert_eq!(
        payload_ptr,
        frame.payload.as_ptr(),
        "decoded payload should reuse buffer memory (zero-copy)"
    );
}

#[test]
fn mysql_wrap_payload_reuses_bytes() {
    use super::examples::MysqlFrameCodec;

    let codec = MysqlFrameCodec::new(128);
    let payload = Bytes::from(vec![3_u8; 10]);
    let frame = codec.wrap_payload(payload.clone());

    assert_eq!(
        payload.as_ref().as_ptr(),
        frame.payload.as_ref().as_ptr(),
        "wrap_payload should reuse the Bytes without copying"
    );
}

#[test]
fn mysql_frame_payload_bytes_reuses_memory() {
    use super::examples::MysqlFrameCodec;

    let codec = MysqlFrameCodec::new(128);
    let payload = Bytes::from(vec![9_u8; 5]);
    let frame = codec.wrap_payload(payload.clone());

    let extracted = MysqlFrameCodec::frame_payload_bytes(&frame);

    assert_eq!(
        frame.payload.as_ptr(),
        extracted.as_ptr(),
        "frame_payload_bytes should return the same memory region"
    );
}
