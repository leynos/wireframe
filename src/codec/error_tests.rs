//! Unit tests for codec error types and recovery policies.

use std::io;

use rstest::rstest;

use super::{CodecError, EofError, FramingError, ProtocolError};
use crate::codec::RecoveryPolicy;

/// Parameterised test for recovery policy defaults and disconnect behaviour.
///
/// Each case specifies:
/// - `error`: the `CodecError` variant to test
/// - `expected_policy`: the expected default recovery policy
/// - `should_disconnect`: whether the error should trigger a disconnect
#[rstest]
#[case::oversized_frame(
    CodecError::Framing(FramingError::OversizedFrame { size: 2000, max: 1024 }),
    RecoveryPolicy::Drop,
    false
)]
#[case::empty_frame(
    CodecError::Framing(FramingError::EmptyFrame),
    RecoveryPolicy::Drop,
    false
)]
#[case::invalid_length_encoding(
    CodecError::Framing(FramingError::InvalidLengthEncoding),
    RecoveryPolicy::Disconnect,
    true
)]
#[case::protocol_error(
    CodecError::Protocol(ProtocolError::UnknownMessageType { type_id: 99 }),
    RecoveryPolicy::Drop,
    false
)]
#[case::io_error(
    CodecError::Io(io::Error::other("test")),
    RecoveryPolicy::Disconnect,
    true
)]
#[case::clean_eof(
    CodecError::Eof(EofError::CleanClose),
    RecoveryPolicy::Disconnect,
    true
)]
#[case::mid_frame_eof(
    CodecError::Eof(EofError::MidFrame { bytes_received: 100, expected: 200 }),
    RecoveryPolicy::Disconnect,
    true
)]
#[case::mid_header_eof(
    CodecError::Eof(EofError::MidHeader { bytes_received: 2, header_size: 4 }),
    RecoveryPolicy::Disconnect,
    true
)]
fn recovery_policy_defaults(
    #[case] error: CodecError,
    #[case] expected_policy: RecoveryPolicy,
    #[case] should_disconnect: bool,
) {
    assert_eq!(error.default_recovery_policy(), expected_policy);
    assert_eq!(error.should_disconnect(), should_disconnect);
}

#[test]
fn clean_eof_is_detectable() {
    let err = CodecError::Eof(EofError::CleanClose);
    assert!(err.is_clean_close());
}

#[test]
fn mid_frame_eof_is_not_clean() {
    let err = CodecError::Eof(EofError::MidFrame {
        bytes_received: 100,
        expected: 200,
    });
    assert!(!err.is_clean_close());
}

#[test]
fn mid_header_eof_is_not_clean() {
    let err = CodecError::Eof(EofError::MidHeader {
        bytes_received: 2,
        header_size: 4,
    });
    assert!(!err.is_clean_close());
}

/// Parameterised test for `CodecError` to `io::Error` conversion.
///
/// Each case specifies:
/// - `error`: the `CodecError` variant to convert
/// - `expected_kind`: the expected `io::ErrorKind` after conversion
#[rstest]
#[case::framing_error(
    CodecError::Framing(FramingError::EmptyFrame),
    io::ErrorKind::InvalidData
)]
#[case::eof_error(
    CodecError::Eof(EofError::MidFrame { bytes_received: 10, expected: 20 }),
    io::ErrorKind::UnexpectedEof
)]
#[case::protocol_error(
    CodecError::Protocol(ProtocolError::UnknownMessageType { type_id: 1 }),
    io::ErrorKind::InvalidData
)]
#[case::io_error(CodecError::Io(io::Error::other("test")), io::ErrorKind::Other)]
fn codec_error_converts_to_io_error_with_correct_kind(
    #[case] error: CodecError,
    #[case] expected_kind: io::ErrorKind,
) {
    let io_err: io::Error = error.into();
    assert_eq!(io_err.kind(), expected_kind);
}

/// Parameterised test for `error_type()` category strings.
///
/// Each case specifies:
/// - `error`: the `CodecError` variant to test
/// - `expected_type`: the expected error type string
#[rstest]
#[case::framing(CodecError::Framing(FramingError::EmptyFrame), "framing")]
#[case::protocol(
    CodecError::Protocol(ProtocolError::UnknownMessageType { type_id: 1 }),
    "protocol"
)]
#[case::io(CodecError::Io(io::Error::other("test")), "io")]
#[case::eof(CodecError::Eof(EofError::CleanClose), "eof")]
fn error_type_returns_correct_category(#[case] error: CodecError, #[case] expected_type: &str) {
    assert_eq!(error.error_type(), expected_type);
}

#[test]
fn framing_error_display_includes_details() {
    let err = FramingError::OversizedFrame {
        size: 2000,
        max: 1024,
    };
    let display = err.to_string();
    assert!(display.contains("2000"));
    assert!(display.contains("1024"));
}

#[test]
fn protocol_error_display_includes_details() {
    let err = ProtocolError::SequenceViolation {
        expected: 5,
        actual: 10,
    };
    let display = err.to_string();
    assert!(display.contains('5'));
    assert!(display.contains("10"));
}

#[test]
fn eof_error_display_includes_byte_counts() {
    let err = EofError::MidFrame {
        bytes_received: 100,
        expected: 200,
    };
    let display = err.to_string();
    assert!(display.contains("100"));
    assert!(display.contains("200"));
}
