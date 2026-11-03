//! Coverage for frame pipeline emission and buffering behaviour.

use std::{
    num::NonZeroUsize,
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    },
    time::Duration,
};

use bytes::Bytes;
use futures::StreamExt;
use rstest::{fixture, rstest};
use tokio::io::DuplexStream;
use tokio_util::codec::Framed;

use super::*;
use crate::{
    app::combined_codec::CombinedCodec,
    codec::{LengthDelimitedFrameCodec, MIN_FRAME_LENGTH},
    serializer::BincodeSerializer,
};

#[path = "codec_driver_tests/fragmentation_properties.rs"]
mod fragmentation_properties;
#[path = "codec_driver_tests/growth_properties.rs"]
mod growth_properties;
#[cfg(feature = "metrics")]
#[path = "codec_driver_tests/send_outcome_metrics.rs"]
mod send_outcome_metrics;

/// Byte appended by the test protocol's `before_send` hook.
const MARKER: u8 = 0xa5;

/// Codec used by the pipeline tests.
type TestCodec = LengthDelimitedFrameCodec;
/// Decoder half paired with [`TestCodec`].
type TestDecoder = <TestCodec as FrameCodec>::Decoder;
/// Encoder half paired with [`TestCodec`].
type TestEncoder = <TestCodec as FrameCodec>::Encoder;
/// Framed duplex stream wired to the test codec.
type TestFramed = Framed<DuplexStream, CombinedCodec<TestDecoder, TestEncoder>>;

/// Framed client/server pair sharing the same codec configuration.
struct FramedPair {
    /// Server half whose frames the pipeline writes.
    server: TestFramed,
    /// Client half that decodes what reached the wire.
    client: TestFramed,
}

/// Build a connected client/server framed pair over a duplex stream.
fn framed_pair() -> FramedPair {
    let codec = TestCodec::default();
    let (client, server) = tokio::io::duplex(1024);
    FramedPair {
        server: Framed::new(server, CombinedCodec::new(codec.decoder(), codec.encoder())),
        client: Framed::new(client, CombinedCodec::new(codec.decoder(), codec.encoder())),
    }
}

/// Build hooks whose `before_send` appends [`MARKER`] and counts calls.
fn marker_hooks(counter: &Arc<AtomicUsize>) -> ProtocolHooks<Bytes, ()> {
    let counter = Arc::clone(counter);
    ProtocolHooks {
        before_send: Some(Box::new(
            move |frame: &mut Bytes, _ctx: &mut ConnectionContext| {
                let mut bytes = frame.to_vec();
                bytes.push(MARKER);
                *frame = Bytes::from(bytes);
                counter.fetch_add(1, Ordering::SeqCst);
            },
        )),
        ..ProtocolHooks::default()
    }
}

/// Build hooks whose `before_send` grows every frame by `extra` bytes.
///
/// Used to pin down what happens when a hook ignores the frame-length
/// contract.
fn growing_hooks(extra: usize) -> ProtocolHooks<Bytes, ()> {
    ProtocolHooks {
        before_send: Some(Box::new(
            move |frame: &mut Bytes, _ctx: &mut ConnectionContext| {
                let mut bytes = frame.to_vec();
                bytes.resize(bytes.len().saturating_add(extra), MARKER);
                *frame = Bytes::from(bytes);
            },
        )),
        ..ProtocolHooks::default()
    }
}

/// Read every frame the server wrote before closing the transport.
async fn collect_frames(client: &mut TestFramed) -> Vec<Bytes> {
    let mut frames = Vec::new();
    while let Some(frame) = client.next().await {
        frames.push(frame.expect("client decode should succeed"));
    }
    frames
}

/// Default unfragmented pipeline shared by the buffering tests.
#[fixture]
fn pipeline() -> FramePipeline {
    let config = None;
    FramePipeline::new(config)
}

/// Processing one envelope buffers exactly one frame with its identifiers.
#[rstest]
fn process_single_envelope_emits_one_frame(mut pipeline: FramePipeline) {
    let env = Envelope::new(1, Some(42), vec![1, 2, 3]);
    pipeline
        .process(env)
        .expect("processing should succeed without fragmentation");
    let mut output = pipeline.drain_output();
    assert_eq!(output.len(), 1);
    let first = output
        .pop()
        .expect("pipeline should emit exactly one envelope");
    assert_eq!(first.id, 1);
    assert_eq!(first.correlation_id, Some(42));
    assert_eq!(first.payload, vec![1, 2, 3]);
}

/// Draining the buffer leaves it empty for the next flush.
#[rstest]
fn drain_clears_buffer(mut pipeline: FramePipeline) {
    pipeline
        .process(Envelope::new(1, None, vec![]))
        .expect("processing should succeed without fragmentation");
    let first = pipeline.drain_output();
    assert_eq!(first.len(), 1);

    let second = pipeline.drain_output();
    assert_eq!(second, [] as [crate::app::Envelope; 0]);
}

/// A pipeline built without a config reports no fragmentation state.
#[rstest]
fn pipeline_without_fragmentation(pipeline: FramePipeline) {
    assert!(!pipeline.has_fragmentation());
}

/// `before_send` mutates every frame flushed from the pipeline.
#[rstest]
#[tokio::test]
async fn before_send_mutates_every_flushed_frame() {
    let counter = Arc::new(AtomicUsize::new(0));
    let mut pipeline: FramePipeline<Bytes> =
        FramePipeline::with_hooks(None, marker_hooks(&counter));
    pipeline
        .process(Envelope::new(1, None, vec![1]))
        .expect("first envelope should process");
    pipeline
        .process(Envelope::new(2, None, vec![2]))
        .expect("second envelope should process");

    let FramedPair {
        mut server,
        mut client,
    } = framed_pair();
    let codec = TestCodec::default();
    let mut transport = OutboundTransport {
        serializer: &BincodeSerializer,
        codec: &codec,
        framed: &mut server,
    };
    flush_pipeline_output(&mut transport, &mut pipeline)
        .await
        .expect("flush should succeed");
    drop(server);

    let frames = collect_frames(&mut client).await;
    assert_eq!(frames.len(), 2, "both envelopes should reach the wire");
    assert!(
        frames.iter().all(|frame| frame.ends_with(&[MARKER])),
        "before_send should mutate every flushed frame"
    );
    assert_eq!(
        counter.load(Ordering::SeqCst),
        2,
        "before_send should run once per flushed frame"
    );
}

/// `before_send` runs once for every fragment of a fragmented envelope.
#[rstest]
#[tokio::test]
async fn before_send_runs_for_each_fragment() {
    let counter = Arc::new(AtomicUsize::new(0));
    let message_cap = NonZeroUsize::new(256).expect("message cap must be non-zero");
    let config = FragmentationConfig::for_frame_budget(96, message_cap, Duration::from_secs(5))
        .expect("frame budget must exceed fragmentation overhead");
    let payload_cap = config.fragment_payload_cap.get();
    let mut pipeline: FramePipeline<Bytes> =
        FramePipeline::with_hooks(Some(config), marker_hooks(&counter));
    pipeline
        .process(Envelope::new(
            1,
            None,
            vec![7; payload_cap.saturating_add(16)],
        ))
        .expect("oversized envelope should fragment");

    let FramedPair {
        mut server,
        mut client,
    } = framed_pair();
    let codec = TestCodec::default();
    let mut transport = OutboundTransport {
        serializer: &BincodeSerializer,
        codec: &codec,
        framed: &mut server,
    };
    flush_pipeline_output(&mut transport, &mut pipeline)
        .await
        .expect("flush should succeed");
    drop(server);

    let frames = collect_frames(&mut client).await;
    assert!(
        frames.len() > 1,
        "fragmentation should produce multiple frames, got {}",
        frames.len()
    );
    assert!(
        frames.iter().all(|frame| frame.ends_with(&[MARKER])),
        "before_send should mutate every fragment"
    );
    assert_eq!(
        counter.load(Ordering::SeqCst),
        frames.len(),
        "before_send should run once per fragment"
    );
}

/// Default hooks leave the encoded frame byte-for-byte unchanged.
#[rstest]
#[tokio::test]
async fn default_hooks_leave_frames_unchanged() {
    let mut pipeline: FramePipeline<Bytes> = FramePipeline::new(None);
    let payload = vec![1, 2, 3];
    pipeline
        .process(Envelope::new(7, Some(9), payload.clone()))
        .expect("envelope should process");

    let FramedPair {
        mut server,
        mut client,
    } = framed_pair();
    let codec = TestCodec::default();
    let mut transport = OutboundTransport {
        serializer: &BincodeSerializer,
        codec: &codec,
        framed: &mut server,
    };
    flush_pipeline_output(&mut transport, &mut pipeline)
        .await
        .expect("flush should succeed");
    drop(server);

    let frames = collect_frames(&mut client).await;
    assert_eq!(frames.len(), 1);
    let frame = frames.first().expect("one frame should reach the wire");
    let (decoded, consumed) = BincodeSerializer
        .deserialize::<Envelope>(frame)
        .expect("frame should decode as an envelope");
    assert_eq!(consumed, frame.len());
    assert_eq!(decoded.id, 7);
    assert_eq!(decoded.payload, payload);
}

/// A hook that pushes a frame past the codec limit must fail the send rather
/// than deliver a truncated or oversized frame.
#[rstest]
#[tokio::test]
async fn before_send_output_over_the_frame_limit_fails_the_send() {
    let mut pipeline: FramePipeline<Bytes> =
        FramePipeline::with_hooks(None, growing_hooks(MIN_FRAME_LENGTH));
    pipeline
        .process(Envelope::new(1, None, vec![1]))
        .expect("envelope should process");

    let codec = TestCodec::new(MIN_FRAME_LENGTH);
    let (client, server) = tokio::io::duplex(1024);
    let mut server = Framed::new(server, CombinedCodec::new(codec.decoder(), codec.encoder()));
    let mut client = Framed::new(client, CombinedCodec::new(codec.decoder(), codec.encoder()));
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

    drop(server);
    assert!(
        collect_frames(&mut client).await.is_empty(),
        "no oversized frame should reach the wire"
    );
}
