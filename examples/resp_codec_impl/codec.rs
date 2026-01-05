//! RESP codec and adapter implementations.

use std::io;

use bytes::{Buf, Bytes, BytesMut};
use tokio_util::codec::{Decoder, Encoder};
use wireframe::codec::FrameCodec;

use super::{
    encode::{encode_frame, encoded_len},
    frame::RespFrame,
    parse::parse_frame,
};

/// RESP frame codec for use with Wireframe.
#[derive(Clone, Debug)]
pub struct RespFrameCodec {
    max_frame_length: usize,
}

impl RespFrameCodec {
    /// Create a new RESP codec with the given maximum frame length.
    #[must_use]
    pub fn new(max_frame_length: usize) -> Self { Self { max_frame_length } }
}

/// Tokio codec adapter for RESP frames.
#[derive(Clone, Debug)]
pub struct RespAdapter {
    max_frame_length: usize,
}

impl RespAdapter {
    fn new(max_frame_length: usize) -> Self { Self { max_frame_length } }
}

impl Decoder for RespAdapter {
    type Item = RespFrame;
    type Error = io::Error;

    fn decode(&mut self, src: &mut BytesMut) -> Result<Option<Self::Item>, Self::Error> {
        let Some((frame, consumed)) = parse_frame(src, self.max_frame_length)? else {
            return Ok(None);
        };
        if consumed > self.max_frame_length {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "frame too large",
            ));
        }
        src.advance(consumed);
        Ok(Some(frame))
    }
}

impl Encoder<RespFrame> for RespAdapter {
    type Error = io::Error;

    fn encode(&mut self, item: RespFrame, dst: &mut BytesMut) -> Result<(), Self::Error> {
        let len = encoded_len(&item)?;
        if len > self.max_frame_length {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "frame too large",
            ));
        }
        dst.reserve(len);
        encode_frame(&item, dst)
    }
}

impl FrameCodec for RespFrameCodec {
    type Frame = RespFrame;
    type Decoder = RespAdapter;
    type Encoder = RespAdapter;

    fn decoder(&self) -> Self::Decoder { RespAdapter::new(self.max_frame_length) }

    fn encoder(&self) -> Self::Encoder { RespAdapter::new(self.max_frame_length) }

    fn frame_payload(frame: &Self::Frame) -> &[u8] {
        match frame {
            RespFrame::BulkString(Some(payload)) => payload.as_slice(),
            _ => &[],
        }
    }

    fn wrap_payload(&self, payload: Bytes) -> Self::Frame {
        RespFrame::BulkString(Some(payload.to_vec()))
    }

    fn max_frame_length(&self) -> usize { self.max_frame_length }
}
