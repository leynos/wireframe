//! Tests for frame handling helpers.

use std::sync::{
    Arc,
    atomic::{AtomicUsize, Ordering},
};

use bytes::{Buf, BufMut, Bytes, BytesMut};
use futures::StreamExt;
use rstest::{fixture, rstest};
use tokio::io::DuplexStream;
use tokio_util::{
    codec::{Decoder, Encoder},
    sync::CancellationToken,
};

use super::ResponseContext;
use crate::{
    app::{
        Envelope,
        codec_driver::{FramePipeline, OutboundTransport, send_envelope},
        combined_codec::CombinedCodec,
    },
    codec::FrameCodec,
    connection::{ConnectionActor, ConnectionChannels},
    hooks::{ConnectionContext, ProtocolHooks},
    push::PushQueues,
    serializer::BincodeSerializer,
};

/// Byte appended to frames by the ordering test's `before_send` hook.
const MARKER: u8 = 0xa5;

/// Test frame carrying a tag byte and payload.
#[derive(Clone, Debug)]
struct TestFrame {
    tag: u8,
    payload: Vec<u8>,
}

/// Test codec that wraps payloads with a distinctive tag byte.
#[derive(Clone, Debug)]
struct TestCodec {
    max_frame_length: usize,
    counter: Arc<AtomicUsize>,
}

impl TestCodec {
    fn new(max_frame_length: usize) -> Self {
        Self {
            max_frame_length,
            counter: Arc::new(AtomicUsize::new(0)),
        }
    }

    fn wraps(&self) -> usize { self.counter.load(Ordering::SeqCst) }
}

#[derive(Clone, Debug)]
struct TestAdapter {
    max_frame_length: usize,
}

impl TestAdapter {
    const fn new(max_frame_length: usize) -> Self { Self { max_frame_length } }
}

impl Decoder for TestAdapter {
    type Item = TestFrame;
    type Error = std::io::Error;

    /// Decode one tag-and-length frame, returning `None` until it is complete.
    fn decode(&mut self, src: &mut BytesMut) -> Result<Option<Self::Item>, Self::Error> {
        const HEADER_LEN: usize = 2;
        if src.len() < HEADER_LEN {
            return Ok(None);
        }

        let mut header = src.as_ref();
        let tag = header.get_u8();
        let payload_len = header.get_u8() as usize;
        if payload_len > self.max_frame_length {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                "payload too large",
            ));
        }
        if src.len() < HEADER_LEN + payload_len {
            return Ok(None);
        }

        let mut frame_bytes = src.split_to(HEADER_LEN + payload_len);
        frame_bytes.advance(HEADER_LEN);
        let payload = frame_bytes.to_vec();

        Ok(Some(TestFrame { tag, payload }))
    }
}

impl Encoder<TestFrame> for TestAdapter {
    type Error = std::io::Error;

    /// Encode one frame as a tag byte, a length byte, and the payload.
    fn encode(&mut self, item: TestFrame, dst: &mut BytesMut) -> Result<(), Self::Error> {
        if item.payload.len() > self.max_frame_length {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidInput,
                "payload too large",
            ));
        }

        let payload_len = u8::try_from(item.payload.len()).map_err(|_| {
            std::io::Error::new(std::io::ErrorKind::InvalidInput, "payload too long")
        })?;
        dst.reserve(2 + item.payload.len());
        dst.put_u8(item.tag);
        dst.put_u8(payload_len);
        dst.extend_from_slice(&item.payload);
        Ok(())
    }
}

impl FrameCodec for TestCodec {
    type Frame = TestFrame;
    type Decoder = TestAdapter;
    type Encoder = TestAdapter;

    fn decoder(&self) -> Self::Decoder { TestAdapter::new(self.max_frame_length) }

    fn encoder(&self) -> Self::Encoder { TestAdapter::new(self.max_frame_length) }

    fn frame_payload(frame: &Self::Frame) -> &[u8] { frame.payload.as_slice() }

    /// Wraps payload with tag byte 0x42 to verify codec-aware wrapping.
    fn wrap_payload(&self, payload: Bytes) -> Self::Frame {
        self.counter.fetch_add(1, Ordering::SeqCst);
        TestFrame {
            tag: 0x42,
            payload: payload.to_vec(),
        }
    }

    fn correlation_id(frame: &Self::Frame) -> Option<u64> { Some(u64::from(frame.tag)) }

    fn max_frame_length(&self) -> usize { self.max_frame_length }
}

struct TestHarness {
    codec: TestCodec,
    framed: tokio_util::codec::Framed<DuplexStream, CombinedCodec<TestAdapter, TestAdapter>>,
    client: DuplexStream,
}

#[fixture]
fn harness() -> TestHarness {
    let max_frame_length = 64;
    build_harness(max_frame_length)
}

#[fixture]
fn small_harness() -> TestHarness {
    let max_frame_length = 4;
    build_harness(max_frame_length)
}

fn build_harness(max_frame_length: usize) -> TestHarness {
    let codec = TestCodec::new(max_frame_length);
    let (client, server) = tokio::io::duplex(256);
    let combined = CombinedCodec::new(codec.decoder(), codec.encoder());
    let framed = tokio_util::codec::Framed::new(server, combined);

    TestHarness {
        codec,
        framed,
        client,
    }
}

/// Verify `send_envelope` uses `F::wrap_payload` to frame responses.
#[rstest]
#[tokio::test]
async fn send_envelope_wraps_with_codec(harness: TestHarness) {
    let TestHarness {
        codec,
        mut framed,
        client,
    } = harness;

    let mut pipeline: FramePipeline<TestFrame> = FramePipeline::new(None);
    let envelope = Envelope::new(1, Some(99), vec![1, 2, 3, 4]);
    let mut transport = OutboundTransport {
        serializer: &BincodeSerializer,
        codec: &codec,
        framed: &mut framed,
    };
    send_envelope(&mut transport, &envelope, &mut pipeline)
        .await
        .expect("send should succeed");

    drop(framed);

    let combined_client = CombinedCodec::new(codec.decoder(), codec.encoder());
    let mut client_framed = tokio_util::codec::Framed::new(client, combined_client);
    let frame = client_framed
        .next()
        .await
        .expect("expected a frame")
        .expect("decode should succeed");

    assert_eq!(frame.tag, 0x42, "wrap_payload should set tag to 0x42");
    assert_eq!(codec.wraps(), 1, "wrap_payload should advance codec state");
}

/// Verify `before_send` observes the codec-wrapped frame, not the raw payload.
///
/// The hook records the tag that [`TestCodec::wrap_payload`] assigns and
/// appends [`MARKER`] to the frame payload. Running the hook before wrapping
/// would leave the tag unset and lose the marker, so the assertions pin the
/// documented ordering: serialize, wrap, hook, send.
#[rstest]
#[tokio::test]
async fn before_send_observes_codec_wrapped_frame(harness: TestHarness) {
    let TestHarness {
        codec,
        mut framed,
        client,
    } = harness;

    let observed_tag = Arc::new(AtomicUsize::new(usize::MAX));
    let hook_tag = Arc::clone(&observed_tag);
    let hooks = ProtocolHooks {
        before_send: Some(Box::new(
            move |frame: &mut TestFrame, _ctx: &mut ConnectionContext| {
                hook_tag.store(usize::from(frame.tag), Ordering::SeqCst);
                frame.payload.push(MARKER);
            },
        )),
        ..ProtocolHooks::default()
    };
    let mut pipeline: FramePipeline<TestFrame> = FramePipeline::with_hooks(None, hooks);
    let envelope = Envelope::new(1, Some(99), vec![1, 2, 3, 4]);
    let mut transport = OutboundTransport {
        serializer: &BincodeSerializer,
        codec: &codec,
        framed: &mut framed,
    };
    send_envelope(&mut transport, &envelope, &mut pipeline)
        .await
        .expect("send should succeed");

    drop(framed);

    let combined_client = CombinedCodec::new(codec.decoder(), codec.encoder());
    let mut client_framed = tokio_util::codec::Framed::new(client, combined_client);
    let frame = client_framed
        .next()
        .await
        .expect("expected a frame")
        .expect("decode should succeed");

    assert_eq!(
        observed_tag.load(Ordering::SeqCst),
        usize::from(0x42u8),
        "before_send should see the codec-wrapped frame"
    );
    assert_eq!(frame.tag, 0x42, "codec wrapping should survive the hook");
    assert!(
        frame.payload.ends_with(&[MARKER]),
        "hook mutation should reach the wire"
    );
}

/// Verify `ResponseContext` fields are accessible and usable.
#[rstest]
#[tokio::test]
async fn response_context_holds_references(harness: TestHarness) {
    let TestHarness {
        codec,
        mut framed,
        client: _client,
    } = harness;
    let serializer = BincodeSerializer;
    let mut pipeline: FramePipeline<TestFrame> = FramePipeline::new(None);
    let (queues, handle) = PushQueues::<Envelope>::builder()
        .high_capacity(1)
        .low_capacity(1)
        .build()
        .expect("actor queues");
    let mut actor = ConnectionActor::new_untracked(
        ConnectionChannels::new(queues, handle),
        None,
        CancellationToken::new(),
        ProtocolHooks::default(),
    );

    let ctx: ResponseContext<'_, BincodeSerializer, _, TestCodec> = ResponseContext {
        serializer: &serializer,
        framed: &mut framed,
        pipeline: &mut pipeline,
        codec: &codec,
        actor: &mut actor,
    };

    // Verify fields are accessible (compile-time check)
    assert!(
        !ctx.pipeline.has_fragmentation(),
        "assertion failed: !ctx.pipeline.has_fragmentation()"
    );
}

/// Verify `send_envelope` returns error on send failure.
#[rstest]
#[tokio::test]
async fn send_envelope_returns_error_on_failure(small_harness: TestHarness) {
    let TestHarness {
        codec,
        mut framed,
        client: _client,
    } = small_harness;

    // Payload exceeds max_frame_length after serialization, so encode will fail
    let oversized_payload = vec![0u8; 100];
    let envelope = Envelope::new(1, Some(99), oversized_payload);
    let mut pipeline: FramePipeline<TestFrame> = FramePipeline::new(None);
    let mut transport = OutboundTransport {
        serializer: &BincodeSerializer,
        codec: &codec,
        framed: &mut framed,
    };
    let result = send_envelope(&mut transport, &envelope, &mut pipeline).await;

    assert!(
        result.is_err(),
        "expected send to fail for oversized payload"
    );
}
