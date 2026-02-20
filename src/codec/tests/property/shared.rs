//! Shared proptest helpers for codec property tests.

use std::{io, ops::Range};

use bytes::{BufMut, BytesMut};
use proptest::{
    collection::vec,
    prelude::{Just, Strategy, any, prop_oneof},
    test_runner::{Config as ProptestConfig, RngAlgorithm, TestCaseError, TestRng, TestRunner},
};

pub fn deterministic_runner(cases: u32) -> TestRunner {
    let config = ProptestConfig {
        cases,
        ..ProptestConfig::default()
    };
    let rng = TestRng::deterministic_rng(RngAlgorithm::ChaCha);
    TestRunner::new_with_rng(config, rng)
}

pub fn boundary_length_strategy(max_frame_length: usize) -> impl Strategy<Value = usize> {
    prop_oneof![
        Just(0usize),
        Just(1usize),
        Just(max_frame_length.saturating_sub(1)),
        Just(max_frame_length),
        0usize..=max_frame_length,
    ]
}

pub fn boundary_payload_strategy(max_frame_length: usize) -> impl Strategy<Value = Vec<u8>> {
    boundary_length_strategy(max_frame_length).prop_flat_map(|len| vec(any::<u8>(), len))
}

pub fn payload_sequence_strategy(
    max_frame_length: usize,
    sequence_lengths: Range<usize>,
) -> impl Strategy<Value = Vec<Vec<u8>>> {
    vec(
        boundary_payload_strategy(max_frame_length),
        sequence_lengths,
    )
}

#[derive(Clone, Debug)]
pub enum MalformedLengthDelimitedInput {
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
    pub fn expected_error_kind(&self) -> io::ErrorKind {
        match self {
            Self::OversizedLength { .. } => io::ErrorKind::InvalidData,
            Self::PartialHeader(_) | Self::TruncatedPayload { .. } => io::ErrorKind::UnexpectedEof,
        }
    }

    pub fn to_bytes(&self) -> Vec<u8> {
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

pub fn malformed_length_delimited_strategy(
    max_frame_length: usize,
    length_header_size: usize,
    oversized_window: usize,
) -> impl Strategy<Value = MalformedLengthDelimitedInput> {
    let partial_header_end = length_header_size.max(2);
    let partial_header = vec(any::<u8>(), 1..partial_header_end)
        .prop_map(MalformedLengthDelimitedInput::PartialHeader);

    let truncated_payload = (1usize..=max_frame_length.max(1))
        .prop_flat_map(|declared_len| (Just(declared_len), vec(any::<u8>(), 0..declared_len)))
        .prop_map(
            |(declared_len, payload)| MalformedLengthDelimitedInput::TruncatedPayload {
                declared_len,
                payload,
            },
        );

    let oversized_min = max_frame_length.saturating_add(1);
    let oversized_max = max_frame_length
        .saturating_add(oversized_window)
        .max(oversized_min);
    let oversized_length = (oversized_min..=oversized_max)
        .prop_map(|declared_len| MalformedLengthDelimitedInput::OversizedLength { declared_len });

    prop_oneof![partial_header, truncated_payload, oversized_length]
}

pub fn mock_payload_strategy(max_frame_length: usize) -> impl Strategy<Value = Vec<u8>> {
    let bounded_max = max_frame_length.min(usize::from(u16::MAX));
    boundary_payload_strategy(bounded_max)
}

pub fn mock_session_strategy(
    max_frame_length: usize,
    payloads_per_session: Range<usize>,
    sessions_per_case: Range<usize>,
) -> impl Strategy<Value = Vec<Vec<Vec<u8>>>> {
    vec(
        vec(
            mock_payload_strategy(max_frame_length),
            payloads_per_session,
        ),
        sessions_per_case,
    )
}

pub fn expected_sequence(index: usize) -> Result<u16, TestCaseError> {
    u16::try_from(index + 1)
        .map_err(|_| TestCaseError::fail("generated index exceeded u16 range".to_owned()))
}

fn usize_to_u32(value: usize) -> u32 { u32::try_from(value).unwrap_or(u32::MAX) }
