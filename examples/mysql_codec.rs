//! Example `MySQL` framing codec wired into a `WireframeApp`.
//!
//! `MySQL` packets use a 4-byte header: 3-byte little-endian payload length and
//! a 1-byte sequence number.

use std::io;

use bytes::{Buf, BufMut, BytesMut};
use tokio_util::codec::{Decoder, Encoder};
use wireframe::{
    BincodeSerializer,
    app::{Envelope, WireframeApp},
    codec::FrameCodec,
};

#[derive(Clone, Debug)]
struct MysqlFrame {
    sequence_id: u8,
    payload: Vec<u8>,
}

#[derive(Clone, Debug)]
struct MysqlFrameCodec {
    max_frame_length: usize,
}

impl MysqlFrameCodec {
    fn new(max_frame_length: usize) -> Self { Self { max_frame_length } }
}

#[derive(Clone, Debug)]
struct MysqlAdapter {
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

        let payload_len = u32::try_from(payload_len)
            .map_err(|_| io::Error::new(io::ErrorKind::InvalidInput, "payload too long"))?;
        let len_lo = u8::try_from(payload_len & 0xff)
            .map_err(|_| io::Error::new(io::ErrorKind::InvalidInput, "payload too long"))?;
        let len_mid = u8::try_from((payload_len >> 8) & 0xff)
            .map_err(|_| io::Error::new(io::ErrorKind::InvalidInput, "payload too long"))?;
        let len_hi = u8::try_from((payload_len >> 16) & 0xff)
            .map_err(|_| io::Error::new(io::ErrorKind::InvalidInput, "payload too long"))?;

        dst.reserve(HEADER_LEN + payload_len as usize);
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

    fn decoder(&self) -> impl Decoder<Item = Self::Frame, Error = io::Error> + Send {
        MysqlAdapter::new(self.max_frame_length)
    }

    fn encoder(&self) -> impl Encoder<Self::Frame, Error = io::Error> + Send {
        MysqlAdapter::new(self.max_frame_length)
    }

    fn frame_payload(frame: &Self::Frame) -> &[u8] { frame.payload.as_slice() }

    fn wrap_payload(payload: Vec<u8>) -> Self::Frame {
        MysqlFrame {
            sequence_id: 0,
            payload,
        }
    }

    fn correlation_id(frame: &Self::Frame) -> Option<u64> { Some(u64::from(frame.sequence_id)) }

    fn max_frame_length(&self) -> usize { self.max_frame_length }
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let app = WireframeApp::<BincodeSerializer, (), Envelope>::new()?
        .with_codec(MysqlFrameCodec::new(1024))
        .route(1, std::sync::Arc::new(|_: &Envelope| Box::pin(async {})))?;

    let _ = app;
    Ok(())
}
