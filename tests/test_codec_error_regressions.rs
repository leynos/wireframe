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
    LengthDelimitedFrameCodec,
    ProtocolError,
    RecoveryPolicy,
    RecoveryPolicyHook,
};
use wireframe_testing::{ObservabilityHandle, decode_frames_with_codec};

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

fn extract_codec_error(error: &std::io::Error) -> &CodecError {
    let mut current = error
        .get_ref()
        .map(|inner| inner as &(dyn std::error::Error + 'static));

    while let Some(err) = current {
        if let Some(codec_error) = err.downcast_ref::<CodecError>() {
            return codec_error;
        }
        current = err.source();
    }

    panic!("decode helper should preserve a wrapped CodecError: {error}");
}

struct TaxonomyCase {
    error: CodecError,
    expected_type: &'static str,
    expected_policy: RecoveryPolicy,
    expected_disconnect: bool,
    expected_clean_close: bool,
}

struct HelperDecodeCase {
    eof_case: DefaultEofCase,
    expected_variant: &'static str,
    expected_policy: RecoveryPolicy,
    expected_fragment: &'static str,
}

#[derive(Clone, Copy, Debug)]
enum DefaultEofCase {
    CleanClose,
    ZeroLengthPayload,
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
            Self::ZeroLengthPayload => vec![0, 0, 0, 0],
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

    fn consumes_complete_frame(self) -> bool {
        matches!(self, Self::CleanClose | Self::ZeroLengthPayload)
    }
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
#[case::mid_header(
    TaxonomyCase {
        error: CodecError::Eof(EofError::MidHeader {
            bytes_received: 2,
            header_size: LENGTH_HEADER_SIZE,
        }),
        expected_type: "eof",
        expected_policy: RecoveryPolicy::Disconnect,
        expected_disconnect: true,
        expected_clean_close: false,
    }
)]
#[case::mid_frame(
    TaxonomyCase {
        error: CodecError::Eof(EofError::MidFrame {
            bytes_received: 3,
            expected: 4,
        }),
        expected_type: "eof",
        expected_policy: RecoveryPolicy::Disconnect,
        expected_disconnect: true,
        expected_clean_close: false,
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
#[case::zero_length_payload(DefaultEofCase::ZeroLengthPayload, EofError::CleanClose)]
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
#[case::mid_header(
    HelperDecodeCase {
        eof_case: DefaultEofCase::MidHeader,
        expected_variant: "eof",
        expected_policy: RecoveryPolicy::Disconnect,
        expected_fragment: "premature EOF during header",
    }
)]
#[case::mid_frame(
    HelperDecodeCase {
        eof_case: DefaultEofCase::MidFrame,
        expected_variant: "eof",
        expected_policy: RecoveryPolicy::Disconnect,
        expected_fragment: "premature EOF: 3 bytes of 4 byte frame received",
    }
)]
fn wireframe_testing_fixture_cases_continue_to_surface_expected_errors(
    #[case] case: HelperDecodeCase,
) {
    let codec = LengthDelimitedFrameCodec::new(4096);
    let error = decode_frames_with_codec(&codec, case.eof_case.wire())
        .expect_err("fixture input should fail to decode");
    let codec_error = extract_codec_error(&error);

    assert_eq!(
        codec_error.error_type(),
        case.expected_variant,
        "wrong codec error category for helper regression"
    );
    assert_eq!(
        codec_error.default_recovery_policy(),
        case.expected_policy,
        "wrong recovery policy for helper regression"
    );

    assert!(
        error.to_string().contains(case.expected_fragment),
        "expected error containing '{}', got: {error}",
        case.expected_fragment
    );
}
