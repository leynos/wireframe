//! Metric coverage for outbound application-response send outcomes.
//!
//! The bounded counter must separate a successful write, a codec rejection
//! caused by hook growth, and a transport failure, because operators use those
//! classes to tell a misbehaving hook apart from a broken peer connection.

use std::{error::Error, future::poll_fn, io, pin::Pin, task::Poll};

use metrics::{SharedString, Unit};
use metrics_util::{
    CompositeKey,
    debugging::{DebugValue, DebuggingRecorder},
};
use tokio::io::{AsyncRead, DuplexStream, ReadBuf};

use super::*;
use crate::{
    message::{DecodeWith, EncodeWith},
    metrics::APP_RESPONSE_SENDS,
    serializer::{MessageCompatibilitySerializer, Serializer},
};

type MetricSample = (CompositeKey, Option<Unit>, Option<SharedString>, DebugValue);

/// Count the recorded samples for one stable outcome label.
fn outcome_count(samples: &[MetricSample], expected: &str) -> u64 {
    samples
        .iter()
        .filter(|(key, ..)| key.key().name() == APP_RESPONSE_SENDS)
        .filter(|(key, ..)| {
            key.key()
                .labels()
                .any(|label| label.key() == "outcome" && label.value() == expected)
        })
        .map(|(_, _, _, value)| match value {
            DebugValue::Counter(count) => *count,
            _ => 0,
        })
        .sum()
}

/// Flush the buffered envelopes through a fresh in-memory transport.
///
/// Returns the flush result, the client half of the duplex stream, and the
/// framed server half. A caller inspecting the client must keep the server
/// half alive: dropping it closes the peer, so a "nothing was written" probe
/// would then observe end-of-stream instead of a pending read.
async fn flush_in_memory<S>(
    serializer: &S,
    codec: &TestCodec,
    pipeline: &mut FramePipeline<Bytes>,
) -> (io::Result<()>, DuplexStream, TestFramed)
where
    S: Serializer + Send + Sync,
    Envelope: EncodeWith<S>,
{
    let (client, server) = tokio::io::duplex(1024);
    let mut framed = Framed::new(server, CombinedCodec::new(codec.decoder(), codec.encoder()));
    let mut transport = OutboundTransport {
        serializer,
        codec,
        framed: &mut framed,
    };
    let result = flush_pipeline_output(&mut transport, pipeline).await;
    (result, client, framed)
}

/// A normal flush records exactly one successful send outcome.
#[test]
fn successful_send_records_success_outcome() {
    let recorder = DebuggingRecorder::new();
    let snapshotter = recorder.snapshotter();

    ::metrics::with_local_recorder(&recorder, || {
        futures::executor::block_on(async {
            let mut pipeline: FramePipeline<Bytes> = FramePipeline::new(None);
            pipeline
                .process(Envelope::new(1, None, vec![1, 2, 3]))
                .expect("envelope should process");
            let codec = TestCodec::default();
            let (result, client, _server) =
                flush_in_memory(&BincodeSerializer, &codec, &mut pipeline).await;
            result.expect("flush should succeed");
            drop(client);
        });
    });

    let samples = snapshotter.snapshot().into_vec();
    assert_eq!(outcome_count(&samples, "success"), 1);
    assert_eq!(outcome_count(&samples, "codec_rejected"), 0);
    assert_eq!(outcome_count(&samples, "transport_failure"), 0);
}

/// Hook growth past the codec limit records a codec rejection.
#[test]
fn hook_growth_past_the_limit_records_codec_rejection() {
    let recorder = DebuggingRecorder::new();
    let snapshotter = recorder.snapshotter();

    ::metrics::with_local_recorder(&recorder, || {
        futures::executor::block_on(async {
            let mut pipeline: FramePipeline<Bytes> =
                FramePipeline::with_hooks(None, growing_hooks(MIN_FRAME_LENGTH));
            pipeline
                .process(Envelope::new(1, None, vec![1]))
                .expect("envelope should process");
            let codec = TestCodec::new(MIN_FRAME_LENGTH);
            let (result, client, _server) =
                flush_in_memory(&BincodeSerializer, &codec, &mut pipeline).await;
            let error = result.expect_err("a frame grown past the codec limit must not be sent");
            assert!(
                error.to_string().contains("frame exceeds max length"),
                "the failure should name the codec limit, got: {error}"
            );
            drop(client);
        });
    });

    let samples = snapshotter.snapshot().into_vec();
    assert_eq!(outcome_count(&samples, "codec_rejected"), 1);
    assert_eq!(outcome_count(&samples, "success"), 0);
    assert_eq!(outcome_count(&samples, "transport_failure"), 0);
}

/// A write to a closed peer records a transport failure.
#[test]
fn closed_peer_records_transport_failure() {
    let recorder = DebuggingRecorder::new();
    let snapshotter = recorder.snapshotter();

    ::metrics::with_local_recorder(&recorder, || {
        futures::executor::block_on(async {
            let mut pipeline: FramePipeline<Bytes> = FramePipeline::new(None);
            pipeline
                .process(Envelope::new(1, None, vec![1, 2, 3]))
                .expect("envelope should process");
            let codec = TestCodec::default();
            let (client, server) = tokio::io::duplex(1024);
            drop(client);
            let mut server_framed =
                Framed::new(server, CombinedCodec::new(codec.decoder(), codec.encoder()));
            let mut transport = OutboundTransport {
                serializer: &BincodeSerializer,
                codec: &codec,
                framed: &mut server_framed,
            };
            flush_pipeline_output(&mut transport, &mut pipeline)
                .await
                .expect_err("writing to a closed peer must fail");
        });
    });

    let samples = snapshotter.snapshot().into_vec();
    assert_eq!(outcome_count(&samples, "transport_failure"), 1);
    assert_eq!(outcome_count(&samples, "success"), 0);
    assert_eq!(outcome_count(&samples, "codec_rejected"), 0);
}

/// Serializer that refuses every value, isolating the serialization stage.
#[derive(Debug)]
struct FailingSerializer;

impl MessageCompatibilitySerializer for FailingSerializer {}

impl Serializer for FailingSerializer {
    fn serialize<M>(&self, _value: &M) -> Result<Vec<u8>, Box<dyn Error + Send + Sync>>
    where
        M: EncodeWith<Self>,
    {
        Err(Box::new(io::Error::other(
            "serializer refused the envelope",
        )))
    }

    fn deserialize<M>(&self, _bytes: &[u8]) -> Result<(M, usize), Box<dyn Error + Send + Sync>>
    where
        M: DecodeWith<Self>,
    {
        Err(Box::new(io::Error::other(
            "failing serializer does not decode",
        )))
    }
}

/// A serializer failure records one failure outcome and writes nothing.
#[test]
fn serialization_failure_records_serialization_failure_outcome() {
    let recorder = DebuggingRecorder::new();
    let snapshotter = recorder.snapshotter();

    ::metrics::with_local_recorder(&recorder, || {
        futures::executor::block_on(async {
            let mut pipeline: FramePipeline<Bytes> = FramePipeline::new(None);
            pipeline
                .process(Envelope::new(1, None, vec![1, 2, 3]))
                .expect("envelope should process");
            let codec = TestCodec::default();
            let (result, mut client, _server) =
                flush_in_memory(&FailingSerializer, &codec, &mut pipeline).await;
            let error = result.expect_err("a serializer failure must fail the flush");
            assert!(
                error
                    .to_string()
                    .contains("serialization error: serializer refused the envelope"),
                "the failure should preserve the serializer error, got: {error}"
            );
            // `DuplexStream` has no `try_read`, so poll the read once: a
            // pending read proves the peer received no bytes.
            let mut probe = [0_u8; 1];
            let mut buf = ReadBuf::new(&mut probe);
            let polled =
                poll_fn(|cx| Poll::Ready(Pin::new(&mut client).poll_read(cx, &mut buf))).await;
            assert!(
                matches!(polled, Poll::Pending),
                "no frame must reach the peer, got: {polled:?}"
            );
        });
    });

    let samples = snapshotter.snapshot().into_vec();
    assert_eq!(outcome_count(&samples, "serialization_failure"), 1);
    assert_eq!(outcome_count(&samples, "success"), 0);
    assert_eq!(outcome_count(&samples, "codec_rejected"), 0);
    assert_eq!(outcome_count(&samples, "transport_failure"), 0);
}
