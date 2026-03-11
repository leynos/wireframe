//! Regression tests for codec error taxonomy and recovery labels backed by
//! `wireframe_testing`.
#![cfg(not(loom))]

use rstest::rstest;
use wireframe::codec::{
    CodecError,
    CodecErrorContext,
    EofError,
    FramingError,
    LENGTH_HEADER_SIZE,
    ProtocolError,
    RecoveryPolicy,
    RecoveryPolicyHook,
};
use wireframe_testing::{
    ObservabilityHandle,
    decode_frames_with_codec,
    oversized_hotline_wire,
    truncated_hotline_header,
};

#[path = "common/codec_error_regression_support.rs"]
mod codec_error_regression_support;

use codec_error_regression_support::{
    StrictRecoveryHook,
    classify_eof,
    encoded_default_frame,
    truncated_payload_wire,
};

fn record_codec_error(obs: &mut ObservabilityHandle, error: &CodecError, policy: RecoveryPolicy) {
    metrics::with_local_recorder(obs.recorder(), || {
        wireframe::metrics::inc_codec_error(error.error_type(), policy.as_str());
    });
    obs.snapshot();
}

struct TaxonomyCase {
    error: CodecError,
    expected_type: &'static str,
    expected_policy: RecoveryPolicy,
    expected_disconnect: bool,
    expected_clean_close: bool,
}

#[derive(Clone, Copy, Debug)]
enum DefaultEofCase {
    CleanClose,
    MidHeader,
    MidFrame,
}

impl DefaultEofCase {
    fn wire(self) -> Vec<u8> {
        match self {
            Self::CleanClose => match encoded_default_frame(&[1, 2, 3, 4]) {
                Ok(wire) => wire,
                Err(error) => panic!("default test codec should encode: {error}"),
            },
            Self::MidHeader => {
                let full = match encoded_default_frame(&[1, 2, 3, 4]) {
                    Ok(wire) => wire,
                    Err(error) => panic!("default test codec should encode: {error}"),
                };
                match full.get(..2) {
                    Some(prefix) => prefix.to_vec(),
                    None => panic!("encoded frame should contain at least two header bytes"),
                }
            }
            Self::MidFrame => match truncated_payload_wire(&[1, 2, 3, 4]) {
                Ok(wire) => wire,
                Err(error) => panic!("partial payload wire should encode: {error}"),
            },
        }
    }

    fn consumes_complete_frame(self) -> bool { matches!(self, Self::CleanClose) }
}

#[rstest]
#[case::oversized(
    TaxonomyCase {
        error: CodecError::Framing(FramingError::OversizedFrame { size: 2048, max: 1024 }),
        expected_type: "framing",
        expected_policy: RecoveryPolicy::Drop,
        expected_disconnect: false,
        expected_clean_close: false,
    }
)]
#[case::invalid_encoding(
    TaxonomyCase {
        error: CodecError::Framing(FramingError::InvalidLengthEncoding),
        expected_type: "framing",
        expected_policy: RecoveryPolicy::Disconnect,
        expected_disconnect: true,
        expected_clean_close: false,
    }
)]
#[case::protocol(
    TaxonomyCase {
        error: CodecError::Protocol(ProtocolError::UnknownMessageType { type_id: 99 }),
        expected_type: "protocol",
        expected_policy: RecoveryPolicy::Drop,
        expected_disconnect: false,
        expected_clean_close: false,
    }
)]
#[case::io(
    TaxonomyCase {
        error: CodecError::Io(std::io::Error::other("connection reset")),
        expected_type: "io",
        expected_policy: RecoveryPolicy::Disconnect,
        expected_disconnect: true,
        expected_clean_close: false,
    }
)]
#[case::clean_close(
    TaxonomyCase {
        error: CodecError::Eof(EofError::CleanClose),
        expected_type: "eof",
        expected_policy: RecoveryPolicy::Disconnect,
        expected_disconnect: true,
        expected_clean_close: true,
    }
)]
fn taxonomy_cases_emit_expected_recovery_labels(#[case] case: TaxonomyCase) {
    assert_eq!(case.error.error_type(), case.expected_type);
    assert_eq!(case.error.default_recovery_policy(), case.expected_policy);
    assert_eq!(case.error.should_disconnect(), case.expected_disconnect);
    assert_eq!(case.error.is_clean_close(), case.expected_clean_close);

    let mut obs = ObservabilityHandle::new();
    obs.clear();
    record_codec_error(&mut obs, &case.error, case.expected_policy);

    assert_eq!(
        obs.codec_error_counter(case.expected_type, case.expected_policy.as_str()),
        1
    );
    assert_eq!(
        obs.counter_without_labels(wireframe::metrics::CODEC_ERRORS),
        1
    );
}

#[rstest]
#[case::clean_close(DefaultEofCase::CleanClose, EofError::CleanClose)]
#[case::mid_header(
    DefaultEofCase::MidHeader,
    EofError::MidHeader {
        bytes_received: 2,
        header_size: LENGTH_HEADER_SIZE,
    }
)]
#[case::mid_frame(
    DefaultEofCase::MidFrame,
    EofError::MidFrame {
        bytes_received: 3,
        expected: 4,
    }
)]
fn default_codec_eof_classification_remains_stable(
    #[case] case: DefaultEofCase,
    #[case] expected: EofError,
) {
    let actual = match classify_eof(&case.wire(), case.consumes_complete_frame()) {
        Ok(actual) => actual,
        Err(error) => panic!("default codec EOF classification should succeed: {error}"),
    };
    assert_eq!(actual, expected);

    let error = CodecError::Eof(actual);
    let mut obs = ObservabilityHandle::new();
    obs.clear();
    record_codec_error(&mut obs, &error, error.default_recovery_policy());

    assert_eq!(obs.codec_error_counter("eof", "disconnect"), 1);
}

#[test]
fn custom_recovery_hook_changes_observed_policy_label() {
    let hook = StrictRecoveryHook;
    let error = CodecError::Framing(FramingError::OversizedFrame {
        size: 1025,
        max: 1024,
    });
    let ctx = CodecErrorContext::new().with_connection_id(7);

    assert_eq!(error.default_recovery_policy(), RecoveryPolicy::Drop);

    let overridden = hook.recovery_policy(&error, &ctx);
    assert_eq!(overridden, RecoveryPolicy::Disconnect);

    let mut obs = ObservabilityHandle::new();
    obs.clear();
    record_codec_error(&mut obs, &error, overridden);

    assert_eq!(obs.codec_error_counter("framing", "disconnect"), 1);
    assert_eq!(obs.codec_error_counter("framing", "drop"), 0);
}

#[rstest]
#[case::oversized(oversized_hotline_wire(64), 64, "payload too large")]
#[case::truncated_header(truncated_hotline_header(), 4096, "bytes remaining")]
fn wireframe_testing_fixture_cases_continue_to_surface_expected_errors(
    #[case] wire: Vec<u8>,
    #[case] max_frame_length: usize,
    #[case] expected_fragment: &str,
) {
    let codec = wireframe::codec::examples::HotlineFrameCodec::new(max_frame_length);
    let error =
        decode_frames_with_codec(&codec, wire).expect_err("fixture input should fail to decode");

    assert!(
        error.to_string().contains(expected_fragment),
        "expected error containing '{expected_fragment}', got: {error}"
    );
}
