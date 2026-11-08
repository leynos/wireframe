//! Tests for Wireframe inbound connection handling.

#[cfg(feature = "metrics")]
use metrics::{SharedString, Unit};
#[cfg(feature = "metrics")]
use metrics_util::{
    CompositeKey,
    debugging::{DebugValue, DebuggingRecorder},
};
use rstest::rstest;
use tokio::io::AsyncWriteExt;
use wireframe_testing::logger;

use super::*;
use crate::{
    app::{connection_runtime::parse_envelope, frame_handling},
    serializer::BincodeSerializer,
    test_helpers::{BadCodec, BadFrame},
};

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
    let result = parse_envelope(&FrameDecodeSerializer(mode), &bytes);
    if trailing == 0 {
        let (decoded, consumed) =
            result.expect("complete envelope must decode through every branch");
        assert_eq!(
            decoded.payload_bytes(),
            &[42],
            "decoded.payload_bytes() should equal &[42]"
        );
        assert_eq!(consumed, bytes.len(), "consumed should equal bytes.len()");
    } else {
        let error = result.expect_err("trailing bytes must fail through every branch");
        assert!(
            error.to_string().contains("trailing bytes after envelope"),
            "assertion failed: error.to_string().contains(\"trailing bytes after envelope\")"
        );
    }
}

#[cfg(feature = "metrics")]
type DecodeRejectionSample = (CompositeKey, Option<Unit>, Option<SharedString>, DebugValue);

#[cfg(feature = "metrics")]
fn has_metric_label(key: &CompositeKey, expected_key: &str, expected_value: &str) -> bool {
    key.key()
        .labels()
        .any(|label| label.key() == expected_key && label.value() == expected_value)
}

#[cfg(feature = "metrics")]
fn is_message_limit_rejection(sample: &DecodeRejectionSample) -> bool {
    let (key, ..) = sample;
    key.key().name() == crate::metrics::DECODE_REJECTIONS
        && key.key().labels().count() == 2
        && has_metric_label(key, "path", "message")
        && has_metric_label(key, "reason", "limit_exceeded")
}

#[cfg(feature = "metrics")]
#[test]
fn oversized_bincode_envelope_counts_one_message_limit_rejection() {
    let declared_length =
        u64::try_from(crate::codec::MAX_FRAME_LENGTH).expect("frame ceiling fits u64") + 1;
    let bytes = bincode::encode_to_vec(
        (1_u32, None::<u64>, declared_length),
        bincode::config::standard(),
    )
    .expect("encode oversized envelope length prefix");
    let recorder = DebuggingRecorder::new();
    let snapshotter = recorder.snapshotter();

    let error =
        metrics::with_local_recorder(&recorder, || parse_envelope(&BincodeSerializer, &bytes))
            .expect_err("oversized envelope must fail");
    assert!(
        matches!(
            error.downcast_ref::<bincode::error::DecodeError>(),
            Some(bincode::error::DecodeError::LimitExceeded)
        ),
        "assertion failed: matches!( error.downcast_ref::<bincode::error::DecodeError>(), \
         Some(bincode::error::DecodeError::LimitExceeded) )"
    );

    let metric_samples = snapshotter.snapshot().into_vec();
    let sample = metric_samples
        .iter()
        .find(|sample| is_message_limit_rejection(sample))
        .expect("message limit rejection must be recorded");
    assert!(
        matches!(
            &sample.3,
            DebugValue::Counter(count) if *count == 1
        ),
        "assertion failed: matches!( &sample.3, DebugValue::Counter(count) if *count == 1 )"
    );
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
            parse_envelope(&app.serializer, BadCodec::frame_payload(&frame)),
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
        parse_envelope(&app.serializer, BadCodec::frame_payload(&frame)),
        &frame,
        &mut failure_tracker,
    )
    .expect_err("expected decode failure to close connection");
    assert_eq!(
        err.kind(),
        io::ErrorKind::InvalidData,
        "err.kind() should equal io::ErrorKind::InvalidData"
    );

    let mut found = false;
    while let Some(record) = log.pop() {
        let message = record.args().to_owned();
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
    assert_eq!(
        error.kind(),
        io::ErrorKind::InvalidData,
        "error.kind() should equal io::ErrorKind::InvalidData"
    );

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
