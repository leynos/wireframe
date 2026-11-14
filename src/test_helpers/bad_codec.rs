//! Deliberately failing frame codec shared by unit and integration tests.
//!
//! The decoder rejects every non-empty input buffer, which lets lifecycle
//! tests drive a connection to a decode error after setup has succeeded
//! without depending on a particular wire encoding.

use std::io;

use bytes::{Bytes, BytesMut};
use tokio_util::codec::{Decoder, Encoder};

use crate::codec::FrameCodec;

/// Frame type produced by [`BadCodec`] when a payload is wrapped.
#[derive(Clone, Debug)]
pub struct BadFrame {
    /// Correlation identifier carried by the frame.
    pub correlation_id: u64,
    /// Raw payload bytes.
    pub payload: Vec<u8>,
}

/// Codec whose decoder always rejects non-empty input.
#[derive(Clone, Debug)]
pub struct BadCodec;

/// Decoder that reports [`io::ErrorKind::InvalidData`] for any pending bytes.
#[derive(Clone, Debug)]
pub struct BadDecoder;

/// Encoder that discards frames; decode failures are the point of this codec.
#[derive(Clone, Debug)]
pub struct BadEncoder;

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
