//! Generated checks for `LengthDelimitedFrameCodec`.

use std::io;

use bytes::{BufMut, Bytes, BytesMut};
use proptest::{
    collection::vec,
    prelude::{Just, Strategy, any, prop_oneof},
    prop_assert,
    prop_assert_eq,
    test_runner::{Config as ProptestConfig, RngAlgorithm, TestCaseError, TestRng, TestRunner},
};
use rstest::rstest;
use tokio_util::codec::{Decoder, Encoder};

use crate::codec::{FrameCodec, LENGTH_HEADER_SIZE, LengthDelimitedFrameCodec};

fn deterministic_runner(cases: u32) -> TestRunner {
    let config = ProptestConfig {
        cases,
        ..ProptestConfig::default()
    };
    let rng = TestRng::deterministic_rng(RngAlgorithm::ChaCha);
    TestRunner::new_with_rng(config, rng)
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
    vec(boundary_payload_strategy(max_frame_length), 1..16)
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
    let oversized_max = max_frame_length.saturating_add(1024).max(oversized_min);
    let oversized_length = (oversized_min..=oversized_max)
        .prop_map(|declared_len| MalformedLengthDelimitedInput::OversizedLength { declared_len });

    prop_oneof![partial_header, truncated_payload, oversized_length]
}

fn usize_to_u32(value: usize) -> u32 { u32::try_from(value).unwrap_or(u32::MAX) }

#[rstest]
#[case(64, 96)]
#[case(256, 128)]
fn generated_length_delimited_sequences_round_trip(
    #[case] max_frame_length: usize,
    #[case] cases: u32,
) {
    let mut runner = deterministic_runner(cases);
    let strategy = payload_sequence_strategy(max_frame_length);

    runner
        .run(&strategy, |payloads| {
            let codec = LengthDelimitedFrameCodec::new(max_frame_length);
            let mut encoder = codec.encoder();
            let mut decoder = codec.decoder();
            let mut wire = BytesMut::new();

            for payload in &payloads {
                let frame = codec.wrap_payload(Bytes::from(payload.clone()));
                encoder
                    .encode(frame, &mut wire)
                    .map_err(|err| TestCaseError::fail(format!("encode failed: {err}")))?;
            }

            for expected in &payloads {
                let frame = decoder
                    .decode(&mut wire)
                    .map_err(|err| TestCaseError::fail(format!("decode failed: {err}")))?
                    .ok_or_else(|| TestCaseError::fail("missing frame during decode".to_owned()))?;

                prop_assert_eq!(LengthDelimitedFrameCodec::frame_payload(&frame), expected);
            }

            prop_assert!(wire.is_empty());
            Ok(())
        })
        .expect("generated default codec sequence should round-trip");
}

#[rstest]
#[case(64, 128)]
#[case(512, 160)]
fn generated_length_delimited_malformed_frames_are_rejected(
    #[case] max_frame_length: usize,
    #[case] cases: u32,
) {
    let mut runner = deterministic_runner(cases);
    let strategy = malformed_length_delimited_strategy(max_frame_length);

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
                        "expected malformed input to fail, got {frame:?}"
                    )));
                }
            }

            Ok(())
        })
        .expect("generated malformed default codec input should fail");
}
