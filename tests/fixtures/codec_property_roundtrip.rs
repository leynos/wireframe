//! `CodecPropertyRoundtripWorld` fixture for rstest-bdd property scenarios.

use std::io;

use bytes::{Buf, BufMut, Bytes, BytesMut};
use proptest::{
    collection::vec,
    prelude::{Just, Strategy, any, prop_oneof},
    prop_assert,
    prop_assert_eq,
    test_runner::{Config as ProptestConfig, RngAlgorithm, TestCaseError, TestRng, TestRunner},
};
use rstest::fixture;
use tokio_util::codec::{Decoder, Encoder};
use wireframe::codec::{FrameCodec, LENGTH_HEADER_SIZE, LengthDelimitedFrameCodec};
/// Re-export `TestResult` from `wireframe_testing` for use in steps.
pub use wireframe_testing::TestResult;

#[derive(Debug, Default)]
pub struct CodecPropertyRoundtripWorld {
    cases: u32,
    default_sequence_checks_passed: bool,
    default_malformed_checks_passed: bool,
    mock_sequence_checks_passed: bool,
}

#[rustfmt::skip]
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
        let strategy = payload_sequence_strategy(max_frame_length);
        let mut runner = deterministic_runner(self.cases);

        runner
            .run(&strategy, |payloads| {
                run_default_sequence_case(payloads.as_slice(), max_frame_length)
            })
            .map_err(|err| format!("default codec generated sequence check failed: {err}"))?;

        self.default_sequence_checks_passed = true;
        Ok(())
    }

    pub fn run_default_malformed_checks(&mut self) -> TestResult {
        let max_frame_length = 256;
        let strategy = malformed_length_delimited_strategy(max_frame_length);
        let mut runner = deterministic_runner(self.cases);

        runner
            .run(&strategy, |input| {
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
            })
            .map_err(|err| format!("default codec malformed check failed: {err}"))?;

        self.default_malformed_checks_passed = true;
        Ok(())
    }

    pub fn run_mock_sequence_checks(&mut self) -> TestResult {
        let max_frame_length = 128;
        let strategy = mock_session_strategy(max_frame_length);
        let mut runner = deterministic_runner(self.cases);

        runner
            .run(&strategy, |sessions| {
                for payloads in &sessions {
                    run_mock_session_case(payloads.as_slice(), max_frame_length)?;
                }
                Ok(())
            })
            .map_err(|err| format!("mock codec generated sequence check failed: {err}"))?;

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
}

fn deterministic_runner(cases: u32) -> TestRunner {
    let config = ProptestConfig {
        cases,
        ..ProptestConfig::default()
    };
    let rng = TestRng::deterministic_rng(RngAlgorithm::ChaCha);
    TestRunner::new_with_rng(config, rng)
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

fn boundary_length_strategy(max_frame_length: usize) -> impl Strategy<Value = usize> {
    prop_oneof![
        Just(0usize),
        Just(1usize),
        Just(max_frame_length.saturating_sub(1)),
        Just(max_frame_length),
        0usize..=max_frame_length,
    ]
}

fn boundary_payload_strategy(max_frame_length: usize) -> impl Strategy<Value = Vec<u8>> {
    boundary_length_strategy(max_frame_length).prop_flat_map(|len| vec(any::<u8>(), len))
}

fn payload_sequence_strategy(max_frame_length: usize) -> impl Strategy<Value = Vec<Vec<u8>>> {
    vec(boundary_payload_strategy(max_frame_length), 1..14)
}

#[derive(Clone, Debug)]
enum MalformedLengthDelimitedInput {
    PartialHeader(Vec<u8>),
    TruncatedPayload {
        declared_len: usize,
        payload: Vec<u8>,
    },
    OversizedLength {
        declared_len: usize,
    },
}

impl MalformedLengthDelimitedInput {
    fn expected_error_kind(&self) -> io::ErrorKind {
        match self {
            Self::OversizedLength { .. } => io::ErrorKind::InvalidData,
            Self::PartialHeader(_) | Self::TruncatedPayload { .. } => io::ErrorKind::UnexpectedEof,
        }
    }

    fn to_bytes(&self) -> Vec<u8> {
        match self {
            Self::PartialHeader(bytes) => bytes.clone(),
            Self::TruncatedPayload {
                declared_len,
                payload,
            } => {
                let mut bytes = BytesMut::new();
                bytes.put_u32(usize_to_u32(*declared_len));
                bytes.extend_from_slice(payload);
                bytes.to_vec()
            }
            Self::OversizedLength { declared_len } => {
                let mut bytes = BytesMut::new();
                bytes.put_u32(usize_to_u32(*declared_len));
                bytes.to_vec()
            }
        }
    }
}

fn malformed_length_delimited_strategy(
    max_frame_length: usize,
) -> impl Strategy<Value = MalformedLengthDelimitedInput> {
    let partial_header = vec(any::<u8>(), 1..LENGTH_HEADER_SIZE)
        .prop_map(MalformedLengthDelimitedInput::PartialHeader);

    let truncated_payload = (1usize..=max_frame_length)
        .prop_flat_map(|declared_len| (Just(declared_len), vec(any::<u8>(), 0..declared_len)))
        .prop_map(
            |(declared_len, payload)| MalformedLengthDelimitedInput::TruncatedPayload {
                declared_len,
                payload,
            },
        );

    let oversized_min = max_frame_length.saturating_add(1);
    let oversized_max = max_frame_length.saturating_add(512).max(oversized_min);
    let oversized_length = (oversized_min..=oversized_max)
        .prop_map(|declared_len| MalformedLengthDelimitedInput::OversizedLength { declared_len });

    prop_oneof![partial_header, truncated_payload, oversized_length]
}

fn usize_to_u32(value: usize) -> u32 { u32::try_from(value).unwrap_or(u32::MAX) }

#[derive(Clone, Debug)]
struct MockStatefulFrame {
    sequence: u16,
    payload: Bytes,
}

#[derive(Clone, Debug)]
struct MockStatefulCodec {
    max_frame_length: usize,
    wrap_sequence: u16,
    encoded_sequence: u16,
    decoded_sequence: u16,
}

impl MockStatefulCodec {
    fn new(max_frame_length: usize) -> Self {
        Self {
            max_frame_length,
            wrap_sequence: 0,
            encoded_sequence: 0,
            decoded_sequence: 0,
        }
    }

    fn wrap_payload(&mut self, payload: Bytes) -> MockStatefulFrame {
        self.wrap_sequence = self.wrap_sequence.wrapping_add(1);
        MockStatefulFrame {
            sequence: self.wrap_sequence,
            payload,
        }
    }

    fn encode(&mut self, frame: &MockStatefulFrame, dst: &mut BytesMut) -> io::Result<()> {
        if frame.payload.len() > self.max_frame_length {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "payload too large",
            ));
        }

        if !is_next_sequence(self.encoded_sequence, frame.sequence) {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "out-of-order sequence",
            ));
        }

        let payload_len = u16::try_from(frame.payload.len())
            .map_err(|_| io::Error::new(io::ErrorKind::InvalidInput, "payload too large"))?;

        dst.reserve(4 + frame.payload.len());
        dst.put_u16(frame.sequence);
        dst.put_u16(payload_len);
        dst.extend_from_slice(&frame.payload);
        self.encoded_sequence = frame.sequence;
        Ok(())
    }

    fn decode(&mut self, src: &mut BytesMut) -> io::Result<Option<MockStatefulFrame>> {
        const HEADER_LEN: usize = 4;
        if src.len() < HEADER_LEN {
            return Ok(None);
        }

        let mut header = src.as_ref();
        let sequence = header.get_u16();
        let payload_len = usize::from(header.get_u16());

        if payload_len > self.max_frame_length {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "payload too large",
            ));
        }

        if !is_next_sequence(self.decoded_sequence, sequence) {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "out-of-order sequence",
            ));
        }

        if src.len() < HEADER_LEN + payload_len {
            return Ok(None);
        }

        let mut frame_bytes = src.split_to(HEADER_LEN + payload_len);
        let sequence = frame_bytes.get_u16();
        let _payload_len = frame_bytes.get_u16();
        let payload = frame_bytes.freeze();
        self.decoded_sequence = sequence;

        Ok(Some(MockStatefulFrame { sequence, payload }))
    }
}

fn is_next_sequence(last_sequence: u16, sequence: u16) -> bool {
    sequence == last_sequence.wrapping_add(1)
}

fn mock_payload_strategy(max_frame_length: usize) -> impl Strategy<Value = Vec<u8>> {
    let bounded_max = max_frame_length.min(usize::from(u16::MAX));
    boundary_payload_strategy(bounded_max)
}

fn mock_session_strategy(max_frame_length: usize) -> impl Strategy<Value = Vec<Vec<Vec<u8>>>> {
    vec(vec(mock_payload_strategy(max_frame_length), 1..10), 1..5)
}

fn run_mock_session_case(
    payloads: &[Vec<u8>],
    max_frame_length: usize,
) -> Result<(), TestCaseError> {
    let mut codec = MockStatefulCodec::new(max_frame_length);
    let mut wire = BytesMut::new();

    for (index, payload) in payloads.iter().enumerate() {
        let frame = codec.wrap_payload(Bytes::from(payload.clone()));
        let expected_sequence = expected_sequence(index)?;
        prop_assert_eq!(frame.sequence, expected_sequence);
        codec
            .encode(&frame, &mut wire)
            .map_err(|err| TestCaseError::fail(format!("stateful encode failed: {err}")))?;
    }

    for (index, payload) in payloads.iter().enumerate() {
        let frame = codec
            .decode(&mut wire)
            .map_err(|err| TestCaseError::fail(format!("decode failed: {err}")))?
            .ok_or_else(|| TestCaseError::fail("missing mock frame".to_owned()))?;

        prop_assert_eq!(frame.sequence, expected_sequence(index)?);
        prop_assert_eq!(frame.payload.as_ref(), payload.as_slice());
    }

    prop_assert!(wire.is_empty());
    Ok(())
}

fn expected_sequence(index: usize) -> Result<u16, TestCaseError> {
    u16::try_from(index + 1)
        .map_err(|_| TestCaseError::fail("generated index exceeded u16 range".to_owned()))
}
