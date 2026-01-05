//! Shared example codecs for tests and feature-gated examples.
//!
//! These implementations are intentionally minimal and meant to illustrate
//! custom framing. They are compiled for tests and for the `examples` feature
//! to avoid duplicating codec logic across example binaries.

use std::io;

use bytes::{Buf, BufMut, Bytes, BytesMut};
use tokio_util::codec::{Decoder, Encoder};

use super::FrameCodec;

#[derive(Clone, Debug)]
pub struct HotlineFrame {
    pub transaction_id: u32,
    pub payload: Vec<u8>,
}

#[derive(Clone, Debug)]
pub struct HotlineFrameCodec {
    max_frame_length: usize,
}

impl HotlineFrameCodec {
    #[must_use]
    pub fn new(max_frame_length: usize) -> Self { Self { max_frame_length } }
}

#[derive(Clone, Debug)]
#[doc(hidden)]
pub struct HotlineAdapter {
    max_frame_length: usize,
}

impl HotlineAdapter {
    fn new(max_frame_length: usize) -> Self { Self { max_frame_length } }
}

impl Decoder for HotlineAdapter {
    type Item = HotlineFrame;
    type Error = io::Error;

    fn decode(&mut self, src: &mut BytesMut) -> Result<Option<Self::Item>, Self::Error> {
        const HEADER_LEN: usize = 20;
        if src.len() < HEADER_LEN {
            return Ok(None);
        }

        let mut header = src.as_ref();
        let data_size = header.get_u32() as usize;
        let total_size = header.get_u32() as usize;
        let transaction_id = header.get_u32();
        if data_size > self.max_frame_length {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "payload too large",
            ));
        }
        if total_size != data_size + HEADER_LEN {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "invalid total size",
            ));
        }
        if src.len() < total_size {
            return Ok(None);
        }

        let mut frame_bytes = src.split_to(total_size);
        frame_bytes.advance(HEADER_LEN);
        let payload = frame_bytes.to_vec();

        Ok(Some(HotlineFrame {
            transaction_id,
            payload,
        }))
    }
}

impl Encoder<HotlineFrame> for HotlineAdapter {
    type Error = io::Error;

    fn encode(&mut self, item: HotlineFrame, dst: &mut BytesMut) -> Result<(), Self::Error> {
        const HEADER_LEN: usize = 20;
        let data_size = item.payload.len();
        if data_size > self.max_frame_length {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "payload too large",
            ));
        }
        let total_size = data_size + HEADER_LEN;
        let data_size = u32::try_from(data_size)
            .map_err(|_| io::Error::new(io::ErrorKind::InvalidInput, "payload too large"))?;
        let total_size = u32::try_from(total_size)
            .map_err(|_| io::Error::new(io::ErrorKind::InvalidInput, "payload too large"))?;
        dst.reserve(total_size as usize);
        dst.put_u32(data_size);
        dst.put_u32(total_size);
        dst.put_u32(item.transaction_id);
        dst.put_bytes(0, 8);
        dst.extend_from_slice(&item.payload);
        Ok(())
    }
}

impl FrameCodec for HotlineFrameCodec {
    type Frame = HotlineFrame;
    type Decoder = HotlineAdapter;
    type Encoder = HotlineAdapter;

    fn decoder(&self) -> Self::Decoder { HotlineAdapter::new(self.max_frame_length) }

    fn encoder(&self) -> Self::Encoder { HotlineAdapter::new(self.max_frame_length) }

    fn frame_payload(frame: &Self::Frame) -> &[u8] { frame.payload.as_slice() }

    fn wrap_payload(&self, payload: Bytes) -> Self::Frame {
        HotlineFrame {
            transaction_id: 0,
            payload: payload.to_vec(),
        }
    }

    fn correlation_id(frame: &Self::Frame) -> Option<u64> { Some(u64::from(frame.transaction_id)) }

    fn max_frame_length(&self) -> usize { self.max_frame_length }
}

#[derive(Clone, Debug)]
pub struct MysqlFrame {
    pub sequence_id: u8,
    pub payload: Vec<u8>,
}

#[derive(Clone, Debug)]
pub struct MysqlFrameCodec {
    max_frame_length: usize,
}

impl MysqlFrameCodec {
    #[must_use]
    pub fn new(max_frame_length: usize) -> Self { Self { max_frame_length } }
}

#[derive(Clone, Debug)]
#[doc(hidden)]
pub struct MysqlAdapter {
    max_frame_length: usize,
}

impl MysqlAdapter {
    fn new(max_frame_length: usize) -> Self { Self { max_frame_length } }
}

impl Decoder for MysqlAdapter {
    type Item = MysqlFrame;
    type Error = io::Error;

    fn decode(&mut self, src: &mut BytesMut) -> Result<Option<Self::Item>, Self::Error> {
        const HEADER_LEN: usize = 4;
        if src.len() < HEADER_LEN {
            return Ok(None);
        }

        let mut header = src.as_ref();
        let len = (header.get_u8() as usize)
            | ((header.get_u8() as usize) << 8)
            | ((header.get_u8() as usize) << 16);
        let sequence_id = header.get_u8();
        if len > self.max_frame_length {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "payload too large",
            ));
        }
        if src.len() < HEADER_LEN + len {
            return Ok(None);
        }

        let mut frame_bytes = src.split_to(HEADER_LEN + len);
        frame_bytes.advance(HEADER_LEN);
        let payload = frame_bytes.to_vec();

        Ok(Some(MysqlFrame {
            sequence_id,
            payload,
        }))
    }
}

impl Encoder<MysqlFrame> for MysqlAdapter {
    type Error = io::Error;

    fn encode(&mut self, item: MysqlFrame, dst: &mut BytesMut) -> Result<(), Self::Error> {
        const HEADER_LEN: usize = 4;
        let payload_len = item.payload.len();
        if payload_len > self.max_frame_length {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "payload too large",
            ));
        }
        if payload_len > 0xff_ff_ff {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "payload too long",
            ));
        }

        #[expect(
            clippy::cast_possible_truncation,
            reason = "bounded by 0xff_ff_ff check above"
        )]
        let payload_len_u32 = payload_len as u32;
        let len_lo = (payload_len_u32 & 0xff) as u8;
        let len_mid = ((payload_len_u32 >> 8) & 0xff) as u8;
        let len_hi = ((payload_len_u32 >> 16) & 0xff) as u8;

        dst.reserve(HEADER_LEN + payload_len);
        dst.put_u8(len_lo);
        dst.put_u8(len_mid);
        dst.put_u8(len_hi);
        dst.put_u8(item.sequence_id);
        dst.extend_from_slice(&item.payload);
        Ok(())
    }
}

impl FrameCodec for MysqlFrameCodec {
    type Frame = MysqlFrame;
    type Decoder = MysqlAdapter;
    type Encoder = MysqlAdapter;

    fn decoder(&self) -> Self::Decoder { MysqlAdapter::new(self.max_frame_length) }

    fn encoder(&self) -> Self::Encoder { MysqlAdapter::new(self.max_frame_length) }

    fn frame_payload(frame: &Self::Frame) -> &[u8] { frame.payload.as_slice() }

    fn wrap_payload(&self, payload: Bytes) -> Self::Frame {
        MysqlFrame {
            sequence_id: 0,
            payload: payload.to_vec(),
        }
    }

    // MySQL sequence_id is a packet counter, not a true correlation ID.
    fn correlation_id(frame: &Self::Frame) -> Option<u64> { Some(u64::from(frame.sequence_id)) }

    fn max_frame_length(&self) -> usize { self.max_frame_length }
}
