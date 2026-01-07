//! Unit tests for codec error types and recovery policies.

use std::io;

use super::{CodecError, EofError, FramingError, ProtocolError};
use crate::codec::RecoveryPolicy;

#[test]
fn oversized_frame_recommends_drop() {
    let err = CodecError::Framing(FramingError::OversizedFrame {
        size: 2000,
        max: 1024,
    });
    assert_eq!(err.default_recovery_policy(), RecoveryPolicy::Drop);
    assert!(!err.should_disconnect());
}

#[test]
fn empty_frame_recommends_drop() {
    let err = CodecError::Framing(FramingError::EmptyFrame);
    assert_eq!(err.default_recovery_policy(), RecoveryPolicy::Drop);
    assert!(!err.should_disconnect());
}

#[test]
fn invalid_length_encoding_recommends_disconnect() {
    let err = CodecError::Framing(FramingError::InvalidLengthEncoding);
    assert_eq!(err.default_recovery_policy(), RecoveryPolicy::Disconnect);
    assert!(err.should_disconnect());
}

#[test]
fn protocol_error_recommends_drop() {
    let err = CodecError::Protocol(ProtocolError::UnknownMessageType { type_id: 99 });
    assert_eq!(err.default_recovery_policy(), RecoveryPolicy::Drop);
    assert!(!err.should_disconnect());
}

#[test]
fn io_error_recommends_disconnect() {
    let err = CodecError::Io(io::Error::other("test"));
    assert_eq!(err.default_recovery_policy(), RecoveryPolicy::Disconnect);
    assert!(err.should_disconnect());
}

#[test]
fn clean_eof_is_detectable() {
    let err = CodecError::Eof(EofError::CleanClose);
    assert!(err.is_clean_close());
    assert!(err.should_disconnect());
}

#[test]
fn mid_frame_eof_recommends_disconnect() {
    let err = CodecError::Eof(EofError::MidFrame {
        bytes_received: 100,
        expected: 200,
    });
    assert!(!err.is_clean_close());
    assert!(err.should_disconnect());
}

#[test]
fn mid_header_eof_recommends_disconnect() {
    let err = CodecError::Eof(EofError::MidHeader {
        bytes_received: 2,
        header_size: 4,
    });
    assert!(!err.is_clean_close());
    assert!(err.should_disconnect());
}

#[test]
fn codec_error_converts_to_io_error_with_correct_kind() {
    let err = CodecError::Framing(FramingError::EmptyFrame);
    let io_err: io::Error = err.into();
    assert_eq!(io_err.kind(), io::ErrorKind::InvalidData);

    let err = CodecError::Eof(EofError::MidFrame {
        bytes_received: 10,
        expected: 20,
    });
    let io_err: io::Error = err.into();
    assert_eq!(io_err.kind(), io::ErrorKind::UnexpectedEof);
}

#[test]
fn error_type_returns_correct_category() {
    assert_eq!(
        CodecError::Framing(FramingError::EmptyFrame).error_type(),
        "framing"
    );
    assert_eq!(
        CodecError::Protocol(ProtocolError::UnknownMessageType { type_id: 1 }).error_type(),
        "protocol"
    );
    assert_eq!(CodecError::Io(io::Error::other("test")).error_type(), "io");
    assert_eq!(CodecError::Eof(EofError::CleanClose).error_type(), "eof");
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
