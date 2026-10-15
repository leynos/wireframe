//! Tests for Wireframe inbound connection handling.

use bytes::{Bytes, BytesMut};
use rstest::rstest;
use tokio::io::AsyncWriteExt;
use tokio_util::codec::{Decoder, Encoder};
use wireframe_testing::logger;

use super::*;
use crate::{app::frame_handling, serializer::BincodeSerializer};

/// The three ways metadata parsing can lead to a fully decoded envelope.
#[derive(Clone, Copy, Debug)]
enum MetadataMode {
    Complete,
    Prefix,
    Fallback,
}

/// Exercise each metadata branch without changing the wire encoding.
struct FrameDecodeSerializer(MetadataMode);

impl crate::serializer::MessageCompatibilitySerializer for FrameDecodeSerializer {}

impl Serializer for FrameDecodeSerializer {
    fn serialize<M>(&self, value: &M) -> Result<Vec<u8>, Box<dyn std::error::Error + Send + Sync>>
    where
        M: EncodeWith<Self>,
    {
        value.encode_with(self)
    }

    fn deserialize<M>(
        &self,
        bytes: &[u8],
    ) -> Result<(M, usize), Box<dyn std::error::Error + Send + Sync>>
    where
        M: DecodeWith<Self>,
    {
        M::decode_with(self, bytes, &crate::message::DeserializeContext::empty())
    }

    fn should_deserialize_after_parse(&self) -> bool { matches!(self.0, MetadataMode::Prefix) }
}

impl FrameMetadata for FrameDecodeSerializer {
    type Frame = Envelope;
    type Error = bincode::error::DecodeError;

    fn parse(&self, bytes: &[u8]) -> Result<(Envelope, usize), Self::Error> {
        match self.0 {
            MetadataMode::Complete => BincodeSerializer.parse(bytes),
            MetadataMode::Prefix => BincodeSerializer
                .parse(bytes)
                .map(|(envelope, _)| (envelope, 1)),
            MetadataMode::Fallback => {
                Err(bincode::error::DecodeError::Other("metadata unavailable"))
            }
        }
    }
}

#[rstest]
#[case::complete_frame(0)]
#[case::trailing_bytes(2)]
fn single_envelope_frames_require_full_consumption(
    #[values(MetadataMode::Complete, MetadataMode::Prefix, MetadataMode::Fallback)]
    mode: MetadataMode,
    #[case] trailing: usize,
) {
    use crate::message::Message;

    let envelope = Envelope::new(1, None, vec![42]);
    let mut bytes = envelope.to_bytes().expect("encode envelope");
    bytes.extend(std::iter::repeat_n(0, trailing));
    let result = core::parse_envelope(&FrameDecodeSerializer(mode), &bytes);
    if trailing == 0 {
        let (decoded, consumed) =
            result.expect("complete envelope must decode through every branch");
        assert_eq!(decoded.payload_bytes(), &[42]);
        assert_eq!(consumed, bytes.len());
    } else {
        let error = result.expect_err("trailing bytes must fail through every branch");
        assert!(error.to_string().contains("trailing bytes after envelope"));
    }
}

#[derive(Clone, Debug)]
struct BadFrame {
    correlation_id: u64,
    payload: Vec<u8>,
}

#[derive(Clone, Debug)]
struct BadCodec;

#[derive(Clone, Debug)]
struct BadDecoder;

#[derive(Clone, Debug)]
struct BadEncoder;

impl Decoder for BadDecoder {
    type Item = BadFrame;
    type Error = io::Error;

    fn decode(&mut self, src: &mut BytesMut) -> Result<Option<Self::Item>, Self::Error> {
        if src.is_empty() {
            Ok(None)
        } else {
            Err(io::Error::new(io::ErrorKind::InvalidData, "bad test frame"))
        }
    }
}

impl Encoder<BadFrame> for BadEncoder {
    type Error = io::Error;

    fn encode(&mut self, _item: BadFrame, _dst: &mut BytesMut) -> Result<(), Self::Error> { Ok(()) }
}

impl FrameCodec for BadCodec {
    type Frame = BadFrame;
    type Decoder = BadDecoder;
    type Encoder = BadEncoder;

    fn decoder(&self) -> Self::Decoder { BadDecoder }

    fn encoder(&self) -> Self::Encoder { BadEncoder }

    fn frame_payload(frame: &Self::Frame) -> &[u8] { frame.payload.as_slice() }

    fn wrap_payload(&self, payload: Bytes) -> Self::Frame {
        BadFrame {
            correlation_id: 0,
            payload: payload.to_vec(),
        }
    }

    fn correlation_id(frame: &Self::Frame) -> Option<u64> { Some(frame.correlation_id) }

    fn max_frame_length(&self) -> usize { 64 }
}

#[test]
fn decode_envelope_tracks_failures_and_logs_correlation_id() {
    let app = WireframeApp::<BincodeSerializer, (), Envelope>::new()
        .expect("build app")
        .with_codec(BadCodec);
    let frame = BadFrame {
        correlation_id: 42,
        payload: vec![0xff],
    };

    let mut log = logger();
    log.clear();
    let mut deser_failures = 0_u32;

    for _ in 1..MAX_DESER_FAILURES {
        let mut failure_tracker =
            frame_handling::DeserFailureTracker::new(&mut deser_failures, MAX_DESER_FAILURES);
        let result = frame_handling::decode_envelope::<BadCodec>(
            core::parse_envelope(&app.serializer, BadCodec::frame_payload(&frame)),
            &frame,
            &mut failure_tracker,
        );
        let decoded = result.expect("expected recoverable decode failure");
        assert!(
            decoded.is_none(),
            "recoverable decode should not yield a message"
        );
    }

    let mut failure_tracker =
        frame_handling::DeserFailureTracker::new(&mut deser_failures, MAX_DESER_FAILURES);
    let err = frame_handling::decode_envelope::<BadCodec>(
        core::parse_envelope(&app.serializer, BadCodec::frame_payload(&frame)),
        &frame,
        &mut failure_tracker,
    )
    .expect_err("expected decode failure to close connection");
    assert_eq!(err.kind(), io::ErrorKind::InvalidData);

    let mut found = false;
    while let Some(record) = log.pop() {
        let message = record.args().to_string();
        if message.contains("failed to decode message")
            && message.contains("correlation_id=Some(42)")
        {
            found = true;
            break;
        }
    }
    assert!(found, "expected correlation_id in decode failure log");
}

#[tokio::test]
async fn unprepared_connection_preserves_stream_errors_and_diagnostics() {
    let app = WireframeApp::<BincodeSerializer, (), Envelope>::new()
        .expect("build app")
        .with_codec(BadCodec);
    let (mut client, server) = tokio::io::duplex(64);
    client.write_all(&[1]).await.expect("write invalid frame");
    drop(client);

    let mut log = logger();
    log.clear();
    let error = app
        .handle_unprepared_connection_result(server)
        .await
        .expect_err("invalid frame must fail the connection");
    assert_eq!(error.kind(), io::ErrorKind::InvalidData);

    let mut found = false;
    while let Some(record) = log.pop() {
        let message = record.args();
        let has_error_context = [
            "connection terminated with error",
            "correlation_id=None",
            "bad test frame",
        ]
        .iter()
        .all(|&fragment| message.contains(fragment));
        if record.level() == log::Level::Warn && has_error_context {
            found = true;
            break;
        }
    }
    assert!(
        found,
        "connection error must retain its warning and context"
    );
}
