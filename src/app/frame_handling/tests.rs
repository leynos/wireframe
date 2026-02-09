//! Tests for frame handling helpers.

use std::sync::{
    Arc,
    atomic::{AtomicUsize, Ordering},
};

use bytes::{Buf, BufMut, Bytes, BytesMut};
use futures::StreamExt;
use rstest::{fixture, rstest};
use tokio::io::DuplexStream;
use tokio_util::codec::{Decoder, Encoder};

use super::{ResponseContext, response::send_response_payload};
use crate::{
    app::{Envelope, combined_codec::CombinedCodec, fragmentation_state::FragmentationState},
    codec::FrameCodec,
};

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
    fn new(max_frame_length: usize) -> Self { Self { max_frame_length } }
}

impl Decoder for TestAdapter {
    type Item = TestFrame;
    type Error = std::io::Error;

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

/// Verify `send_response_payload` uses `F::wrap_payload` to frame responses.
#[rstest]
#[tokio::test]
async fn send_response_payload_wraps_with_codec(harness: TestHarness) {
    let TestHarness {
        codec,
        mut framed,
        client,
    } = harness;

    let payload = vec![1, 2, 3, 4];
    let response = Envelope::new(1, Some(99), payload.clone());
    send_response_payload::<TestCodec, _>(
        &codec,
        &mut framed,
        Bytes::from(payload.clone()),
        &response,
    )
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
    assert_eq!(frame.payload, payload, "payload should match");
    assert_eq!(codec.wraps(), 1, "wrap_payload should advance codec state");
}

/// Verify `ResponseContext` fields are accessible and usable.
#[rstest]
#[tokio::test]
async fn response_context_holds_references(harness: TestHarness) {
    use crate::serializer::BincodeSerializer;

    let TestHarness {
        codec,
        mut framed,
        client: _client,
    } = harness;
    let serializer = BincodeSerializer;
    let mut fragmentation: Option<FragmentationState> = None;

    let ctx: ResponseContext<'_, BincodeSerializer, _, TestCodec> = ResponseContext {
        serializer: &serializer,
        framed: &mut framed,
        fragmentation: &mut fragmentation,
        codec: &codec,
    };

    // Verify fields are accessible (compile-time check with runtime assertion)
    assert!(ctx.fragmentation.is_none());
}

/// Verify `send_response_payload` returns error on send failure.
#[rstest]
#[tokio::test]
async fn send_response_payload_returns_error_on_failure(small_harness: TestHarness) {
    let TestHarness {
        codec,
        mut framed,
        client: _client,
    } = small_harness;

    // Payload exceeds max_frame_length, so encode will fail
    let oversized_payload = vec![0u8; 100];
    let response = Envelope::new(1, Some(99), oversized_payload.clone());
    let result = send_response_payload::<TestCodec, _>(
        &codec,
        &mut framed,
        Bytes::from(oversized_payload),
        &response,
    )
    .await;

    assert!(
        result.is_err(),
        "expected send to fail for oversized payload"
    );
}
