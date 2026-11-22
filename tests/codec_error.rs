//! Integration tests for the `CodecError` taxonomy and recovery policies.

#![forbid(unsafe_code)]

use std::io;

use rstest::rstest;
use wireframe::{
    WireframeError,
    codec::{
        CodecError,
        CodecErrorContext,
        EofError,
        FramingError,
        ProtocolError,
        RecoveryConfig,
        RecoveryPolicy,
        RecoveryPolicyHook,
    },
};

// ============================================================================
// CodecError taxonomy tests
// ============================================================================

#[test]
fn framing_error_oversized_recommends_drop() {
    let err = CodecError::Framing(FramingError::OversizedFrame {
        size: 2000,
        max: 1024,
    });
    assert_eq!(err.default_recovery_policy(), RecoveryPolicy::Drop);
    assert!(!err.should_disconnect());
    assert_eq!(err.error_type(), "framing");
}

#[test]
fn framing_error_invalid_encoding_recommends_disconnect() {
    let err = CodecError::Framing(FramingError::InvalidLengthEncoding);
    assert_eq!(err.default_recovery_policy(), RecoveryPolicy::Disconnect);
    assert!(err.should_disconnect());
}

#[test]
fn protocol_error_recommends_drop() {
    let err = CodecError::Protocol(ProtocolError::UnknownMessageType { type_id: 99 });
    assert_eq!(err.default_recovery_policy(), RecoveryPolicy::Drop);
    assert!(!err.should_disconnect());
    assert_eq!(err.error_type(), "protocol");
}

#[test]
fn io_error_recommends_disconnect() {
    let err = CodecError::Io(io::Error::other("connection reset"));
    assert_eq!(err.default_recovery_policy(), RecoveryPolicy::Disconnect);
    assert!(err.should_disconnect());
    assert_eq!(err.error_type(), "io");
}

#[test]
fn eof_clean_close_is_detectable() {
    let err = CodecError::Eof(EofError::CleanClose);
    assert!(err.is_clean_close());
    assert!(err.should_disconnect());
    assert_eq!(err.error_type(), "eof");
}

#[test]
fn eof_mid_frame_is_not_clean() {
    let err = CodecError::Eof(EofError::MidFrame {
        bytes_received: 100,
        expected: 200,
    });
    assert!(!err.is_clean_close());
    assert!(err.should_disconnect());
}

#[test]
fn codec_error_converts_to_io_error() {
    let err = CodecError::Framing(FramingError::EmptyFrame);
    let io_err: io::Error = err.into();
    assert_eq!(io_err.kind(), io::ErrorKind::InvalidData);

    let mid_frame_err = CodecError::Eof(EofError::MidFrame {
        bytes_received: 10,
        expected: 20,
    });
    let mid_frame_io_err: io::Error = mid_frame_err.into();
    assert_eq!(mid_frame_io_err.kind(), io::ErrorKind::UnexpectedEof);
}

// ============================================================================
// CodecErrorContext tests
// ============================================================================

#[test]
fn context_builder_sets_all_fields() {
    let ctx = CodecErrorContext::new()
        .with_connection_id(42)
        .with_correlation_id(123)
        .with_codec_state("seq=5");

    assert_eq!(ctx.connection_id, Some(42));
    assert_eq!(ctx.correlation_id, Some(123));
    assert_eq!(ctx.codec_state, Some("seq=5".to_owned()));
}

#[test]
fn context_with_peer_address() {
    use std::net::SocketAddr;

    let addr: SocketAddr = "127.0.0.1:8080".parse().expect("valid test address");
    let ctx = CodecErrorContext::new().with_peer_address(addr);
    assert_eq!(ctx.peer_address, Some(addr));
}

// ============================================================================
// RecoveryPolicy tests
// ============================================================================

#[test]
fn recovery_policy_default_is_drop() {
    assert_eq!(RecoveryPolicy::default(), RecoveryPolicy::Drop);
}

#[test]
fn recovery_config_builder() {
    use std::time::Duration;

    let config = RecoveryConfig::default()
        .max_consecutive_drops(5)
        .quarantine_duration(Duration::from_mins(1))
        .log_dropped_frames(false);

    assert_eq!(config.max_consecutive_drops, 5);
    assert_eq!(config.quarantine_duration, Duration::from_mins(1));
    assert!(!config.log_dropped_frames);
}

// ============================================================================
// Custom RecoveryPolicyHook tests
// ============================================================================

struct StrictRecovery;

impl RecoveryPolicyHook for StrictRecovery {
    fn recovery_policy(&self, _error: &CodecError, _ctx: &CodecErrorContext) -> RecoveryPolicy {
        RecoveryPolicy::Disconnect
    }
}

#[test]
fn custom_hook_overrides_default_policy() {
    let hook = StrictRecovery;
    let ctx = CodecErrorContext::new();

    // Oversized frame normally would be Drop
    let err = CodecError::Framing(FramingError::OversizedFrame { size: 100, max: 50 });
    assert_eq!(err.default_recovery_policy(), RecoveryPolicy::Drop);

    // But our strict hook says Disconnect
    assert_eq!(hook.recovery_policy(&err, &ctx), RecoveryPolicy::Disconnect);
}

// NOTE: Decoder EOF and oversized frame tests are covered in src/codec.rs unit tests.
// This integration test file focuses on taxonomy, recovery policy, and error type
// integration tests that exercise the public API from a consumer perspective.

// ============================================================================
// WireframeError integration tests
// ============================================================================

/// The wrapper's own polarity is what is under test here: only a codec clean
/// close may report a clean close, while every other variant must report
/// `false`. Covering only the `true` case would let a wrapper that always
/// answers `true` pass the suite.
#[rstest]
#[case::clean_close(
    WireframeError::<()>::from_codec(CodecError::Eof(EofError::CleanClose)),
    true
)]
#[case::mid_frame(
    WireframeError::<()>::from_codec(CodecError::Eof(EofError::MidFrame {
        bytes_received: 100,
        expected: 200,
    })),
    false
)]
#[case::mid_header(
    WireframeError::<()>::from_codec(CodecError::Eof(EofError::MidHeader {
        bytes_received: 2,
        header_size: 4,
    })),
    false
)]
#[case::framing(
    WireframeError::<()>::from_codec(CodecError::Framing(
        FramingError::InvalidLengthEncoding,
    )),
    false
)]
#[case::io(WireframeError::<()>::from_io(io::Error::other("reset")), false)]
#[case::protocol(WireframeError::<()>::from(()), false)]
#[case::duplicate_route(WireframeError::<()>::DuplicateRoute(7), false)]
#[case::codec_io(
    WireframeError::<()>::from_codec(CodecError::Io(io::Error::other("reset"))),
    false
)]
#[case::codec_protocol(
    WireframeError::<()>::from_codec(CodecError::Protocol(ProtocolError::UnknownMessageType {
        type_id: 99,
    })),
    false
)]
fn wireframe_error_is_clean_close(#[case] error: WireframeError<()>, #[case] expected: bool) {
    assert_eq!(
        error.is_clean_close(),
        expected,
        "unexpected clean-close classification for {error}"
    );
}

#[test]
fn wireframe_error_codec_variant_displays_correctly() {
    let codec_err = CodecError::Framing(FramingError::OversizedFrame {
        size: 2000,
        max: 1024,
    });
    let wf_err: WireframeError<()> = WireframeError::from_codec(codec_err);

    let display = wf_err.to_string();
    assert!(display.contains("codec error"));
    assert!(display.contains("framing error"));
}

#[test]
fn send_error_from_codec() {
    use wireframe::app::error::SendError;

    let codec_err = CodecError::Io(io::Error::other("test"));
    let send_err: SendError = codec_err.into();

    let display = send_err.to_string();
    assert!(display.contains("codec error"));
}
