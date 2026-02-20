//! `CodecPropertyRoundtripWorld` fixture for rstest-bdd property scenarios.

use std::{fmt::Debug, io};

use bytes::{Bytes, BytesMut};
use proptest::{prop_assert, prop_assert_eq, strategy::Strategy, test_runner::TestCaseError};
use rstest::fixture;
use tokio_util::codec::{Decoder, Encoder};
use wireframe::codec::{
    FrameCodec as FrameCodecForTests,
    LENGTH_HEADER_SIZE,
    LengthDelimitedFrameCodec,
};

// NOTE: This fixture depends on property-test helpers sourced from
// `src/codec/tests/property/*` via explicit paths. If these modules move, keep
// these imports aligned with the required API:
// - `MockStatefulCodec`
// - `deterministic_runner`
// - `expected_sequence`
// - `malformed_length_delimited_strategy`
// - `mock_session_strategy`
// - `payload_sequence_strategy`
#[path = "../../src/codec/tests/property/mock_stateful_codec.rs"]
mod property_mock_stateful_codec;
#[path = "../../src/codec/tests/property/shared.rs"]
mod property_shared;

use property_mock_stateful_codec::MockStatefulCodec;
use property_shared::{
    deterministic_runner,
    expected_sequence,
    malformed_length_delimited_strategy,
    mock_session_strategy,
    payload_sequence_strategy,
};
/// Re-export `TestResult` from `wireframe_testing` for use in steps.
pub use wireframe_testing::TestResult;

#[derive(Debug, Default)]
pub struct CodecPropertyRoundtripWorld {
    cases: u32,
    default_sequence_checks_passed: bool,
    default_malformed_checks_passed: bool,
    mock_sequence_checks_passed: bool,
}

#[fixture]
pub fn codec_property_roundtrip_world() -> CodecPropertyRoundtripWorld {
    CodecPropertyRoundtripWorld::default()
}

impl CodecPropertyRoundtripWorld {
    pub fn configure_cases(&mut self, cases: usize) -> TestResult {
        if cases == 0 {
            return Err("generated case count must be greater than zero".into());
        }
        self.cases = u32::try_from(cases)?;
        Ok(())
    }

    pub fn run_default_sequence_checks(&mut self) -> TestResult {
        let max_frame_length = 256;
        self.run_generated_checks(
            payload_sequence_strategy(max_frame_length, 1..14),
            |payloads| run_default_sequence_case(payloads.as_slice(), max_frame_length),
            "default codec generated sequence check failed",
        )?;

        self.default_sequence_checks_passed = true;
        Ok(())
    }

    pub fn run_default_malformed_checks(&mut self) -> TestResult {
        let max_frame_length = 256;
        self.run_generated_checks(
            malformed_length_delimited_strategy(max_frame_length, LENGTH_HEADER_SIZE, 512),
            |input| {
                let codec = LengthDelimitedFrameCodec::new(max_frame_length);
                let mut decoder = codec.decoder();
                let encoded = input.to_bytes();
                let mut buffer = BytesMut::from(encoded.as_slice());

                match decoder.decode_eof(&mut buffer) {
                    Err(err) => prop_assert_eq!(err.kind(), input.expected_error_kind()),
                    Ok(frame) => {
                        return Err(TestCaseError::fail(format!(
                            "expected malformed frame to fail, got {frame:?}"
                        )));
                    }
                }

                Ok(())
            },
            "default codec malformed check failed",
        )?;

        self.default_malformed_checks_passed = true;
        Ok(())
    }

    pub fn run_mock_sequence_checks(&mut self) -> TestResult {
        let max_frame_length = 128;
        self.run_generated_checks(
            mock_session_strategy(max_frame_length, 1..10, 1..5),
            |sessions| {
                for payloads in &sessions {
                    run_mock_session_case(payloads.as_slice(), max_frame_length)?;
                }
                Ok(())
            },
            "mock codec generated sequence check failed",
        )?;

        self.mock_sequence_checks_passed = true;
        Ok(())
    }

    pub fn assert_default_sequence_checks_passed(&self) -> TestResult {
        if !self.default_sequence_checks_passed {
            return Err("default codec sequence checks did not pass".into());
        }
        Ok(())
    }

    pub fn assert_default_malformed_checks_passed(&self) -> TestResult {
        if !self.default_malformed_checks_passed {
            return Err("default codec malformed checks did not pass".into());
        }
        Ok(())
    }

    pub fn assert_mock_sequence_checks_passed(&self) -> TestResult {
        if !self.mock_sequence_checks_passed {
            return Err("mock codec sequence checks did not pass".into());
        }
        Ok(())
    }

    fn run_generated_checks<Value, GeneratedStrategy, CaseFn>(
        &self,
        strategy: GeneratedStrategy,
        run_case: CaseFn,
        context: &str,
    ) -> TestResult
    where
        Value: Debug,
        GeneratedStrategy: Strategy<Value = Value>,
        CaseFn: Fn(Value) -> Result<(), TestCaseError>,
    {
        if self.cases == 0 {
            return Err("generated case count must be configured before running checks".into());
        }

        let mut runner = deterministic_runner(self.cases);
        runner
            .run(&strategy, run_case)
            .map_err(|err| format!("{context}: {err}"))?;
        Ok(())
    }
}

fn run_default_sequence_case(
    payloads: &[Vec<u8>],
    max_frame_length: usize,
) -> Result<(), TestCaseError> {
    let codec = LengthDelimitedFrameCodec::new(max_frame_length);
    let mut encoder = codec.encoder();
    let mut decoder = codec.decoder();
    let mut wire = BytesMut::new();

    for payload in payloads {
        let frame = codec.wrap_payload(Bytes::from(payload.clone()));
        encoder
            .encode(frame, &mut wire)
            .map_err(|err| TestCaseError::fail(format!("encode failed: {err}")))?;
    }

    for expected in payloads {
        let frame = decode_required_default_frame(&mut decoder, &mut wire)?;
        prop_assert_eq!(LengthDelimitedFrameCodec::frame_payload(&frame), expected);
    }

    prop_assert!(wire.is_empty());
    Ok(())
}

fn decode_required_default_frame(
    decoder: &mut impl Decoder<Item = Bytes, Error = io::Error>,
    wire: &mut BytesMut,
) -> Result<Bytes, TestCaseError> {
    decoder
        .decode(wire)
        .map_err(|err| TestCaseError::fail(format!("decode failed: {err}")))?
        .ok_or_else(|| TestCaseError::fail("missing frame during decode".to_owned()))
}

fn run_mock_session_case(
    payloads: &[Vec<u8>],
    max_frame_length: usize,
) -> Result<(), TestCaseError> {
    let root_codec = MockStatefulCodec::new(max_frame_length);
    let connection_codec = root_codec.clone();
    let mut encoder = connection_codec.encoder();
    let mut decoder = connection_codec.decoder();
    let mut wire = BytesMut::new();

    for (index, payload) in payloads.iter().enumerate() {
        let frame = connection_codec.wrap_payload(Bytes::from(payload.clone()));
        let expected_sequence = expected_sequence(index)?;
        prop_assert_eq!(frame.sequence, expected_sequence);
        encoder
            .encode(frame, &mut wire)
            .map_err(|err| TestCaseError::fail(format!("stateful encode failed: {err}")))?;
    }

    for (index, payload) in payloads.iter().enumerate() {
        let frame = decoder
            .decode(&mut wire)
            .map_err(|err| TestCaseError::fail(format!("decode failed: {err}")))?
            .ok_or_else(|| TestCaseError::fail("missing mock frame".to_owned()))?;

        prop_assert_eq!(frame.sequence, expected_sequence(index)?);
        prop_assert_eq!(frame.payload.as_ref(), payload.as_slice());
    }

    prop_assert!(wire.is_empty());
    Ok(())
}
