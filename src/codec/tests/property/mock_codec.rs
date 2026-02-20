//! Generated checks for a mock stateful protocol codec.
use std::io;

use bytes::{BufMut, Bytes, BytesMut};
use proptest::{
    prelude::{Just, Strategy, prop_oneof},
    prop_assert,
    prop_assert_eq,
    test_runner::TestCaseError,
};
use rstest::rstest;
use tokio_util::codec::{Decoder, Encoder};

use super::{
    FrameCodecForTests as FrameCodec,
    mock_stateful_codec::{MockStatefulCodec, MockStatefulFrame},
    shared::{
        deterministic_runner,
        expected_sequence,
        mock_payload_strategy,
        mock_session_strategy,
    },
};

#[derive(Clone, Debug)]
struct OutOfOrderSequenceInput {
    valid_prefix_len: u16,
    invalid_sequence: u16,
    payload: Vec<u8>,
}

fn out_of_order_sequence_strategy(
    max_frame_length: usize,
) -> impl Strategy<Value = OutOfOrderSequenceInput> {
    (0u16..8u16, mock_payload_strategy(max_frame_length))
        .prop_flat_map(|(valid_prefix_len, payload)| {
            let expected_next = valid_prefix_len.saturating_add(1);
            (
                Just(valid_prefix_len),
                prop_oneof![
                    Just(0u16),
                    (1u16..64u16).prop_filter("sequence must break ordering", move |sequence| {
                        *sequence != expected_next
                    }),
                ],
                Just(payload),
            )
        })
        .prop_map(
            |(valid_prefix_len, invalid_sequence, payload)| OutOfOrderSequenceInput {
                valid_prefix_len,
                invalid_sequence,
                payload,
            },
        )
}

fn push_raw_mock_frame(
    dst: &mut BytesMut,
    sequence: u16,
    payload: &[u8],
) -> Result<(), TestCaseError> {
    let payload_len = u16::try_from(payload.len()).map_err(|_| {
        TestCaseError::fail("payload length exceeded u16 during test setup".to_owned())
    })?;
    dst.put_u16(sequence);
    dst.put_u16(payload_len);
    dst.extend_from_slice(payload);
    Ok(())
}

#[rstest]
#[case(64, 96)]
#[case(256, 128)]
fn generated_mock_codec_sequences_round_trip_and_reset_per_connection(
    #[case] max_frame_length: usize,
    #[case] cases: u32,
) {
    let mut runner = deterministic_runner(cases);
    let strategy = mock_session_strategy(max_frame_length, 1..10, 1..5);

    runner
        .run(&strategy, |sessions| {
            let root_codec = MockStatefulCodec::new(max_frame_length);

            for payloads in &sessions {
                let connection_codec = root_codec.clone();
                let mut encoder = connection_codec.encoder();
                let mut decoder = connection_codec.decoder();
                let mut wire = BytesMut::new();

                for (index, payload) in payloads.iter().enumerate() {
                    let frame = connection_codec.wrap_payload(Bytes::from(payload.clone()));
                    let expected_seq = expected_sequence(index)?;
                    prop_assert_eq!(frame.sequence, expected_seq);

                    encoder.encode(frame, &mut wire).map_err(|err| {
                        TestCaseError::fail(format!("stateful encoder failed: {err}"))
                    })?;
                }

                for (index, payload) in payloads.iter().enumerate() {
                    let frame = decoder
                        .decode(&mut wire)
                        .map_err(|err| TestCaseError::fail(format!("decode failed: {err}")))?
                        .ok_or_else(|| TestCaseError::fail("missing mock frame".to_owned()))?;

                    let expected_seq = expected_sequence(index)?;
                    prop_assert_eq!(frame.sequence, expected_seq);
                    prop_assert_eq!(frame.payload.as_ref(), payload.as_slice());
                }

                prop_assert!(wire.is_empty());
            }

            Ok(())
        })
        .expect("generated mock codec sessions should round-trip");
}

#[rstest]
#[case(64, 96)]
fn generated_mock_decoder_rejects_out_of_order_sequences(
    #[case] max_frame_length: usize,
    #[case] cases: u32,
) {
    let mut runner = deterministic_runner(cases);
    let strategy = out_of_order_sequence_strategy(max_frame_length);

    runner
        .run(&strategy, |input| {
            let mut decoder = MockStatefulCodec::new(max_frame_length).decoder();
            let mut wire = BytesMut::new();

            for sequence in 1..=input.valid_prefix_len {
                push_raw_mock_frame(&mut wire, sequence, &[0xab])?;
            }
            push_raw_mock_frame(&mut wire, input.invalid_sequence, &input.payload)?;

            for _ in 0..input.valid_prefix_len {
                let _ = decoder
                    .decode(&mut wire)
                    .map_err(|err| TestCaseError::fail(format!("decode failed early: {err}")))?
                    .ok_or_else(|| {
                        TestCaseError::fail("missing frame in valid prefix".to_owned())
                    })?;
            }

            match decoder.decode(&mut wire) {
                Err(err) => prop_assert_eq!(err.kind(), io::ErrorKind::InvalidData),
                Ok(frame) => {
                    return Err(TestCaseError::fail(format!(
                        "expected sequence error, got {frame:?}"
                    )));
                }
            }

            Ok(())
        })
        .expect("generated out-of-order decoder sequence should fail");
}

#[rstest]
#[case(64, 96)]
fn generated_mock_encoder_rejects_out_of_order_sequences(
    #[case] max_frame_length: usize,
    #[case] cases: u32,
) {
    let mut runner = deterministic_runner(cases);
    let strategy = out_of_order_sequence_strategy(max_frame_length);

    runner
        .run(&strategy, |input| {
            let mut encoder = MockStatefulCodec::new(max_frame_length).encoder();
            let mut wire = BytesMut::new();

            for sequence in 1..=input.valid_prefix_len {
                encoder
                    .encode(
                        MockStatefulFrame {
                            sequence,
                            payload: Bytes::from_static(&[0xcc]),
                        },
                        &mut wire,
                    )
                    .map_err(|err| {
                        TestCaseError::fail(format!(
                            "encoder failed while encoding valid prefix: {err}"
                        ))
                    })?;
            }

            match encoder.encode(
                MockStatefulFrame {
                    sequence: input.invalid_sequence,
                    payload: Bytes::from(input.payload),
                },
                &mut wire,
            ) {
                Err(err) => prop_assert_eq!(err.kind(), io::ErrorKind::InvalidData),
                Ok(()) => {
                    return Err(TestCaseError::fail(
                        "expected stateful encoder to reject sequence".to_owned(),
                    ));
                }
            }

            Ok(())
        })
        .expect("generated out-of-order encoder sequence should fail");
}
