//! Test frame codec fixtures shared across unit and integration tests.

use std::{
    io,
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    },
};

use bytes::{Buf, BufMut, Bytes, BytesMut};
use tokio_util::codec::{Decoder, Encoder};

use crate::codec::FrameCodec;

/// Test frame that wraps payloads with a distinctive tag byte.
#[derive(Clone, Debug)]
pub struct TestFrame {
    /// Tag byte stored in the frame header.
    pub tag: u8,
    /// Payload bytes carried by the frame.
    pub payload: Vec<u8>,
}

/// Codec implementation that wraps payloads with a tagged test frame.
#[derive(Clone, Debug)]
pub struct TestCodec {
    max_frame_length: usize,
    counter: Arc<AtomicUsize>,
}

impl TestCodec {
    /// Create a new test codec with the given maximum frame length.
    #[must_use]
    pub fn new(max_frame_length: usize) -> Self {
        Self {
            max_frame_length,
            counter: Arc::new(AtomicUsize::new(0)),
        }
    }

    /// Return how many payloads have been wrapped.
    #[must_use]
    pub fn wraps(&self) -> usize { self.counter.load(Ordering::SeqCst) }
}

/// Adapter implementing the codec's encoder/decoder pair.
#[derive(Clone, Debug)]
pub struct TestAdapter {
    max_frame_length: usize,
}

impl TestAdapter {
    /// Create a new adapter with the given maximum frame length.
    #[must_use]
    pub fn new(max_frame_length: usize) -> Self { Self { max_frame_length } }
}

impl Decoder for TestAdapter {
    type Item = TestFrame;
    type Error = io::Error;

    fn decode(&mut self, src: &mut BytesMut) -> Result<Option<Self::Item>, Self::Error> {
        const HEADER_LEN: usize = 2;
        if src.len() < HEADER_LEN {
            return Ok(None);
        }

        let mut header = src.as_ref();
        let tag = header.get_u8();
        let payload_len = header.get_u8() as usize;
        if payload_len > self.max_frame_length {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
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
    type Error = io::Error;

    fn encode(&mut self, item: TestFrame, dst: &mut BytesMut) -> Result<(), Self::Error> {
        if item.payload.len() > self.max_frame_length {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "payload too large",
            ));
        }

        let payload_len = u8::try_from(item.payload.len())
            .map_err(|_| io::Error::new(io::ErrorKind::InvalidInput, "payload too long"))?;
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
