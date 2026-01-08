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
