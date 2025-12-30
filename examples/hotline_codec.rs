//! Example Hotline framing codec wired into a `WireframeApp`.
//!
//! The Hotline header is 20 bytes with big-endian lengths and a transaction
//! identifier. This example keeps the frame type protocol-aware while still
//! exposing the raw payload for Wireframe routing.

use std::io;

use bytes::{Buf, BufMut, BytesMut};
use tokio_util::codec::{Decoder, Encoder};
use wireframe::{
    BincodeSerializer,
    app::{Envelope, WireframeApp},
    codec::FrameCodec,
};

#[derive(Clone, Debug)]
struct HotlineFrame {
    transaction_id: u32,
    payload: Vec<u8>,
}

#[derive(Clone, Debug)]
struct HotlineFrameCodec {
    max_frame_length: usize,
}

impl HotlineFrameCodec {
    fn new(max_frame_length: usize) -> Self { Self { max_frame_length } }
}

#[derive(Clone, Debug)]
struct HotlineAdapter {
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
        for _ in 0..8 {
            dst.put_u8(0);
        }
        dst.extend_from_slice(&item.payload);
        Ok(())
    }
}

impl FrameCodec for HotlineFrameCodec {
    type Frame = HotlineFrame;

    fn decoder(&self) -> impl Decoder<Item = Self::Frame, Error = io::Error> + Send {
        HotlineAdapter::new(self.max_frame_length)
    }

    fn encoder(&self) -> impl Encoder<Self::Frame, Error = io::Error> + Send {
        HotlineAdapter::new(self.max_frame_length)
    }

    fn frame_payload(frame: &Self::Frame) -> &[u8] { frame.payload.as_slice() }

    fn wrap_payload(payload: Vec<u8>) -> Self::Frame {
        HotlineFrame {
            transaction_id: 0,
            payload,
        }
    }

    fn correlation_id(frame: &Self::Frame) -> Option<u64> { Some(u64::from(frame.transaction_id)) }

    fn max_frame_length(&self) -> usize { self.max_frame_length }
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let app = WireframeApp::<BincodeSerializer, (), Envelope>::new()?
        .with_codec(HotlineFrameCodec::new(1024))
        .route(1, std::sync::Arc::new(|_: &Envelope| Box::pin(async {})))?;

    let _ = app;
    Ok(())
}
