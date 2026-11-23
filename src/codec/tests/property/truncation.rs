//! Generated truncation classification for the length-delimited decoder.
//!
//! A peer can stop sending at any byte of a frame, so the decoder's answer must
//! depend only on the counts it actually observes: how much of the length
//! prefix arrived, the length that prefix declares, how many payload bytes
//! followed, and the connection's frame limit. These properties drive every
//! combination through `decode` and then `decode_eof`, which is the sequence
//! `FramedRead` performs when the peer half-closes, and check that the
//! structured condition carries the exact counts a protocol hook receives.

use bytes::BytesMut;
use proptest::{
    collection::vec,
    prelude::{Just, Strategy, any, prop_oneof},
    test_runner::TestCaseError,
};
use rstest::rstest;
use tokio_util::codec::Decoder;

use super::shared::deterministic_runner;
use crate::{
    byte_order::write_network_u32,
    codec::{EofError, FrameCodec, LENGTH_HEADER_SIZE, LengthDelimitedFrameCodec, eof_condition},
};

/// One frame a peer abandoned, described by exactly what arrived.
#[derive(Clone, Debug)]
enum Truncation {
    /// Fewer prefix bytes than a length header needs.
    Header(Vec<u8>),
    /// A complete header and some or all of the payload it declares.
    Frame {
        /// Payload length the header declares.
        declared: usize,
        /// Payload bytes that actually arrived.
        payload: Vec<u8>,
    },
}

impl Truncation {
    /// The bytes the peer sent, in wire order.
    fn wire(&self) -> Vec<u8> {
        match self {
            Self::Header(bytes) => bytes.clone(),
            Self::Frame { declared, payload } => {
                let prefix = u32::try_from(*declared).unwrap_or(u32::MAX);
                let mut wire = write_network_u32(prefix).to_vec();
                wire.extend_from_slice(payload);
                wire
            }
        }
    }
}

/// Every truncation the decoder must classify, for one frame limit.
fn truncation_strategy(max_frame_length: usize) -> impl Strategy<Value = Truncation> {
    // A short header carries no payload: the peer stopped before the prefix
    // declared anything, so there is nothing to send next.
    let short_header = vec(any::<u8>(), 0..LENGTH_HEADER_SIZE).prop_map(Truncation::Header);
    let complete_header = (1_usize..=max_frame_length).prop_flat_map(move |declared| {
        // Equal lengths make the frame complete; anything shorter truncates
        // it, which keeps both classifications in the same generated space.
        let payload_len = prop_oneof![Just(declared), 0..declared];
        payload_len.prop_flat_map(move |len| {
            vec(any::<u8>(), len).prop_map(move |payload| Truncation::Frame { declared, payload })
        })
    });
    prop_oneof![short_header, complete_header]
}

/// Classify a truncation against the frame limit it was generated for.
///
/// `None` means the peer stopped at a frame boundary, so nothing was lost.
fn expected_condition(truncation: &Truncation) -> Option<EofError> {
    match truncation {
        Truncation::Header(bytes) if bytes.is_empty() => None,
        Truncation::Header(bytes) => Some(EofError::MidHeader {
            bytes_received: bytes.len(),
            header_size: LENGTH_HEADER_SIZE,
        }),
        Truncation::Frame { declared, payload } if payload.len() == *declared => None,
        Truncation::Frame { declared, payload } => Some(EofError::MidFrame {
            bytes_received: payload.len(),
            expected: *declared,
        }),
    }
}

/// Fail with a generated-case message rather than a bare assertion.
fn check(condition: bool, message: impl FnOnce() -> String) -> Result<(), TestCaseError> {
    if condition {
        Ok(())
    } else {
        Err(TestCaseError::fail(message()))
    }
}

#[rstest]
#[case(16, 96)]
#[case(64, 128)]
#[case(256, 96)]
fn generated_truncations_report_the_exact_counts(
    #[case] max_frame_length: usize,
    #[case] cases: u32,
) {
    let mut runner = deterministic_runner(cases);
    let strategy = truncation_strategy(max_frame_length);

    runner
        .run(&strategy, |truncation| {
            let codec = LengthDelimitedFrameCodec::new(max_frame_length);
            let mut decoder = codec.decoder();
            let wire = truncation.wire();
            let mut buffer = BytesMut::from(wire.as_slice());
            let received = buffer.clone();

            let decoded = decoder
                .decode(&mut buffer)
                .map_err(|error| TestCaseError::fail(format!("decode failed: {error}")))?;
            check(
                decoded.is_none() || truncation_is_complete(&truncation),
                || format!("only a complete frame may decode: {truncation:?}"),
            )?;

            let decode_error = decoder.decode_eof(&mut buffer).err();
            let expected = expected_condition(&truncation);
            check(decode_error.is_some() == expected.is_some(), || {
                format!("unexpected end-of-file outcome for {truncation:?}: {decode_error:?}")
            })?;
            let Some(problem) = decode_error else {
                check(buffer.is_empty(), || {
                    format!("a complete frame must be consumed: {buffer:?}")
                })?;
                return Ok(());
            };

            check(eof_condition(&problem) == expected, || {
                format!(
                    "the structured condition must carry the counts for {truncation:?}: got {:?}, \
                     expected {expected:?}",
                    eof_condition(&problem)
                )
            })?;
            // The hook reports exactly the bytes still held for the frame, so
            // a truncation must leave all of them in place.
            check(buffer == received, || {
                format!("a truncated frame must not be consumed: {buffer:?}")
            })
        })
        .expect("generated truncations should classify exactly");
}

/// Whether the generated frame arrived in full.
fn truncation_is_complete(truncation: &Truncation) -> bool {
    matches!(truncation, Truncation::Frame { declared, payload } if payload.len() == *declared)
}
