//! Integration tests for the `CodecError` taxonomy and recovery policies.

use std::io;

use bytes::{Bytes, BytesMut};
use tokio_util::codec::Decoder;
use wireframe::{
    FrameCodec,
    codec::{
        CodecError,
        CodecErrorContext,
        EofError,
        FramingError,
        LengthDelimitedFrameCodec,
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

    let err = CodecError::Eof(EofError::MidFrame {
        bytes_received: 10,
        expected: 20,
    });
    let io_err: io::Error = err.into();
    assert_eq!(io_err.kind(), io::ErrorKind::UnexpectedEof);
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
    assert_eq!(ctx.codec_state, Some("seq=5".to_string()));
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
        .quarantine_duration(Duration::from_secs(60))
        .log_dropped_frames(false);

    assert_eq!(config.max_consecutive_drops, 5);
    assert_eq!(config.quarantine_duration, Duration::from_secs(60));
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

// ============================================================================
// LengthDelimitedDecoder EOF handling tests
// ============================================================================

/// Helper to test EOF error behaviour for the length-delimited decoder.
///
/// Creates a decoder with `max_frame_length=128`, initialises a buffer from the
/// given bytes, calls `decode_eof`, and asserts:
/// - The result is an error
/// - The error kind is `UnexpectedEof`
/// - The error message contains `expected_error_substring`
///
/// # Panics
///
/// Panics if `decode_eof` returns `Ok`, or if the error kind or message does
/// not match expectations.
fn assert_decode_eof_error(
    initial_buffer: &[u8],
    expected_error_substring: &str,
    test_description: &str,
) {
    let codec = LengthDelimitedFrameCodec::new(128);
    let mut decoder = codec.decoder();
    let mut buf = BytesMut::from(initial_buffer);

    let Err(err) = decoder.decode_eof(&mut buf) else {
        panic!("{test_description}: expected error, got Ok");
    };
    assert_eq!(
        err.kind(),
        io::ErrorKind::UnexpectedEof,
        "{test_description}: expected UnexpectedEof"
    );
    assert!(
        err.to_string().contains(expected_error_substring),
        "{test_description}: error message should contain '{expected_error_substring}', got: {err}"
    );
}

#[test]
fn decoder_eof_empty_buffer_returns_clean_close() {
    // Empty buffer: connection closed cleanly at frame boundary
    assert_decode_eof_error(&[], "connection closed", "clean close");
}

#[test]
fn decoder_eof_partial_header_returns_mid_header() {
    // Only 2 bytes of the 4-byte length prefix header
    assert_decode_eof_error(&[0x00, 0x10], "header", "mid-header EOF");
}

#[test]
fn decoder_eof_partial_payload_returns_mid_frame() {
    // 4-byte header (0x00000010 = 16 bytes payload), but only 4 bytes of payload present
    assert_decode_eof_error(
        &[0x00, 0x00, 0x00, 0x10, 0x01, 0x02, 0x03, 0x04],
        "EOF",
        "mid-frame EOF",
    );
}

#[test]
fn decoder_eof_complete_frame_succeeds() {
    use tokio_util::codec::Encoder;

    let codec = LengthDelimitedFrameCodec::new(128);
    let mut enc = codec.encoder();
    let mut dec = codec.decoder();

    let payload = Bytes::from(vec![1_u8, 2, 3, 4]);
    let frame = payload.clone();

    let mut buf = BytesMut::new();
    enc.encode(frame, &mut buf).expect("encode");

    let result = dec.decode_eof(&mut buf).expect("decode").expect("frame");
    assert_eq!(result.as_ref(), payload.as_ref());
}

// ============================================================================
// Encoder oversized frame produces structured error
// ============================================================================

#[test]
fn encoder_rejects_oversized_with_framing_error() {
    use tokio_util::codec::Encoder;
    use wireframe::codec::MIN_FRAME_LENGTH;

    let codec = LengthDelimitedFrameCodec::new(MIN_FRAME_LENGTH);
    let mut encoder = codec.encoder();

    let payload = Bytes::from(vec![0_u8; MIN_FRAME_LENGTH + 1]);
    let mut buf = BytesMut::new();

    let err = encoder
        .encode(payload, &mut buf)
        .expect_err("expected oversized error");
    // Should be InvalidData (converted from FramingError::OversizedFrame)
    assert_eq!(err.kind(), io::ErrorKind::InvalidData);
}

// ============================================================================
// WireframeError integration tests
// ============================================================================

#[test]
fn wireframe_error_from_codec_method() {
    use wireframe::WireframeError;

    let codec_err = CodecError::Eof(EofError::CleanClose);
    let wf_err: WireframeError<()> = WireframeError::from_codec(codec_err);

    assert!(wf_err.is_clean_close());
}

#[test]
fn wireframe_error_codec_variant_displays_correctly() {
    use wireframe::WireframeError;

    let codec_err = CodecError::Framing(FramingError::OversizedFrame {
        size: 2000,
        max: 1024,
    });
    let wf_err: WireframeError<()> = WireframeError::from_codec(codec_err);

    let display = wf_err.to_string();
    assert!(display.contains("codec error"));
    assert!(display.contains("framing error"));
}

// ============================================================================
// App error WireframeError tests
// ============================================================================

#[test]
fn app_wireframe_error_from_codec() {
    use wireframe::app::error::WireframeError;

    let codec_err = CodecError::Framing(FramingError::EmptyFrame);
    let wf_err: WireframeError = codec_err.into();

    let display = wf_err.to_string();
    assert!(display.contains("codec error"));
}

#[test]
fn send_error_from_codec() {
    use wireframe::app::error::SendError;

    let codec_err = CodecError::Io(io::Error::other("test"));
    let send_err: SendError = codec_err.into();

    let display = send_err.to_string();
    assert!(display.contains("codec error"));
}
