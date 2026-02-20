//! Shared mock stateful codec used across property and BDD tests.

use std::{
    io,
    sync::atomic::{AtomicU16, Ordering},
};

use bytes::{Buf, BufMut, Bytes, BytesMut};
use tokio_util::codec::{Decoder, Encoder};

use super::FrameCodecForTests;

#[derive(Clone, Debug)]
pub(super) struct MockStatefulFrame {
    pub(super) sequence: u16,
    pub(super) payload: Bytes,
}

#[derive(Debug)]
pub(super) struct MockStatefulCodec {
    max_frame_length: usize,
    sequence_counter: AtomicU16,
}

impl MockStatefulCodec {
    pub(super) fn new(max_frame_length: usize) -> Self {
        Self {
            max_frame_length,
            sequence_counter: AtomicU16::new(0),
        }
    }
}

impl Clone for MockStatefulCodec {
    fn clone(&self) -> Self {
        Self {
            max_frame_length: self.max_frame_length,
            sequence_counter: AtomicU16::new(0),
        }
    }
}

#[derive(Clone, Debug)]
pub(super) struct MockStatefulAdapter {
    max_frame_length: usize,
    last_sequence: u16,
}

impl MockStatefulAdapter {
    fn new(max_frame_length: usize) -> Self {
        Self {
            max_frame_length,
            last_sequence: 0,
        }
    }
}

fn is_next_sequence(last_sequence: u16, sequence: u16) -> bool {
    sequence == last_sequence.wrapping_add(1)
}

impl Decoder for MockStatefulAdapter {
    type Item = MockStatefulFrame;
    type Error = io::Error;

    fn decode(&mut self, src: &mut BytesMut) -> Result<Option<Self::Item>, Self::Error> {
        const HEADER_LEN: usize = 4;
        if src.len() < HEADER_LEN {
            return Ok(None);
        }

        let mut header = src.as_ref();
        let sequence = header.get_u16();
        let payload_len = usize::from(header.get_u16());

        if payload_len > self.max_frame_length {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "payload too large",
            ));
        }

        if !is_next_sequence(self.last_sequence, sequence) {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "out-of-order sequence",
            ));
        }

        if src.len() < HEADER_LEN + payload_len {
            return Ok(None);
        }

        let mut frame_bytes = src.split_to(HEADER_LEN + payload_len);
        let sequence = frame_bytes.get_u16();
        let _payload_len = frame_bytes.get_u16();
        let payload = frame_bytes.freeze();
        self.last_sequence = sequence;

        Ok(Some(MockStatefulFrame { sequence, payload }))
    }

    fn decode_eof(&mut self, src: &mut BytesMut) -> Result<Option<Self::Item>, Self::Error> {
        match self.decode(src) {
            Ok(Some(frame)) => Ok(Some(frame)),
            Ok(None) if src.is_empty() => Ok(None),
            Ok(None) => Err(io::Error::new(
                io::ErrorKind::UnexpectedEof,
                "truncated stateful frame",
            )),
            Err(err) => Err(err),
        }
    }
}

impl Encoder<MockStatefulFrame> for MockStatefulAdapter {
    type Error = io::Error;

    fn encode(&mut self, item: MockStatefulFrame, dst: &mut BytesMut) -> Result<(), Self::Error> {
        if item.payload.len() > self.max_frame_length {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "payload too large",
            ));
        }

        if !is_next_sequence(self.last_sequence, item.sequence) {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "out-of-order sequence",
            ));
        }

        let payload_len = u16::try_from(item.payload.len())
            .map_err(|_| io::Error::new(io::ErrorKind::InvalidInput, "payload too large"))?;

        dst.reserve(4 + item.payload.len());
        dst.put_u16(item.sequence);
        dst.put_u16(payload_len);
        dst.extend_from_slice(&item.payload);
        self.last_sequence = item.sequence;

        Ok(())
    }
}

impl FrameCodecForTests for MockStatefulCodec {
    type Frame = MockStatefulFrame;
    type Decoder = MockStatefulAdapter;
    type Encoder = MockStatefulAdapter;

    fn decoder(&self) -> Self::Decoder { MockStatefulAdapter::new(self.max_frame_length) }

    fn encoder(&self) -> Self::Encoder { MockStatefulAdapter::new(self.max_frame_length) }

    fn frame_payload(frame: &Self::Frame) -> &[u8] { frame.payload.as_ref() }

    fn frame_payload_bytes(frame: &Self::Frame) -> Bytes { frame.payload.clone() }

    fn wrap_payload(&self, payload: Bytes) -> Self::Frame {
        let sequence = self
            .sequence_counter
            .fetch_add(1, Ordering::SeqCst)
            .wrapping_add(1);
        MockStatefulFrame { sequence, payload }
    }

    fn max_frame_length(&self) -> usize { self.max_frame_length }
}
