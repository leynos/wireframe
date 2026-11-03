//! Metric coverage for outbound application-response send outcomes.
//!
//! The bounded counter must separate a successful write, a codec rejection
//! caused by hook growth, and a transport failure, because operators use those
//! classes to tell a misbehaving hook apart from a broken peer connection.

use metrics::{SharedString, Unit};
use metrics_util::{
    CompositeKey,
    debugging::{DebugValue, DebuggingRecorder},
};

use super::*;
use crate::metrics::APP_RESPONSE_SENDS;

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
            let FramedPair { mut server, client } = framed_pair();
            let codec = TestCodec::default();
            let mut transport = OutboundTransport {
                serializer: &BincodeSerializer,
                codec: &codec,
                framed: &mut server,
            };
            flush_pipeline_output(&mut transport, &mut pipeline)
                .await
                .expect("flush should succeed");
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
            let (client, server) = tokio::io::duplex(1024);
            let mut server =
                Framed::new(server, CombinedCodec::new(codec.decoder(), codec.encoder()));
            let mut transport = OutboundTransport {
                serializer: &BincodeSerializer,
                codec: &codec,
                framed: &mut server,
            };
            let error = flush_pipeline_output(&mut transport, &mut pipeline)
                .await
                .expect_err("a frame grown past the codec limit must not be sent");
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
            let mut server =
                Framed::new(server, CombinedCodec::new(codec.decoder(), codec.encoder()));
            let mut transport = OutboundTransport {
                serializer: &BincodeSerializer,
                codec: &codec,
                framed: &mut server,
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
