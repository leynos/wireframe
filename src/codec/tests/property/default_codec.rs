//! Generated checks for `LengthDelimitedFrameCodec`.

use bytes::{Bytes, BytesMut};
use proptest::{prop_assert, prop_assert_eq, test_runner::TestCaseError};
use rstest::rstest;
use tokio_util::codec::{Decoder, Encoder};

use super::shared::{
    deterministic_runner,
    malformed_length_delimited_strategy,
    payload_sequence_strategy,
};
use crate::codec::{FrameCodec, LENGTH_HEADER_SIZE, LengthDelimitedFrameCodec};

#[rstest]
#[case(64, 96)]
#[case(256, 128)]
fn generated_length_delimited_sequences_round_trip(
    #[case] max_frame_length: usize,
    #[case] cases: u32,
) {
    let mut runner = deterministic_runner(cases);
    let strategy = payload_sequence_strategy(max_frame_length, 1..16);

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
    let strategy = malformed_length_delimited_strategy(max_frame_length, LENGTH_HEADER_SIZE, 1024);

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
