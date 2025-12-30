//! Integration coverage for custom `FrameCodec` implementations.

use std::{io, sync::Arc};

use bytes::{Buf, BufMut, BytesMut};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio_util::codec::{Decoder, Encoder};
use wireframe::{
    Serializer,
    app::{Envelope, Packet, WireframeApp},
    codec::FrameCodec,
    serializer::BincodeSerializer,
};

#[derive(Clone, Debug)]
struct TaggedFrame {
    tag: u8,
    payload: Vec<u8>,
}

#[derive(Clone, Debug)]
struct TaggedFrameCodec {
    max_frame_length: usize,
}

impl TaggedFrameCodec {
    fn new(max_frame_length: usize) -> Self { Self { max_frame_length } }
}

#[derive(Clone, Debug)]
struct TaggedAdapter {
    max_frame_length: usize,
}

impl TaggedAdapter {
    fn new(max_frame_length: usize) -> Self { Self { max_frame_length } }
}

impl Decoder for TaggedAdapter {
    type Item = TaggedFrame;
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

        Ok(Some(TaggedFrame { tag, payload }))
    }
}

impl Encoder<TaggedFrame> for TaggedAdapter {
    type Error = io::Error;

    fn encode(&mut self, item: TaggedFrame, dst: &mut BytesMut) -> Result<(), Self::Error> {
        if item.payload.len() > self.max_frame_length {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "payload too large",
            ));
        }
        if item.payload.len() > u8::MAX as usize {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "payload too long",
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

impl FrameCodec for TaggedFrameCodec {
    type Frame = TaggedFrame;

    fn decoder(&self) -> impl Decoder<Item = Self::Frame, Error = io::Error> + Send {
        TaggedAdapter::new(self.max_frame_length)
    }

    fn encoder(&self) -> impl Encoder<Self::Frame, Error = io::Error> + Send {
        TaggedAdapter::new(self.max_frame_length)
    }

    fn frame_payload(frame: &Self::Frame) -> &[u8] { frame.payload.as_slice() }

    fn wrap_payload(payload: Vec<u8>) -> Self::Frame { TaggedFrame { tag: 0x1, payload } }

    fn correlation_id(frame: &Self::Frame) -> Option<u64> { Some(u64::from(frame.tag)) }

    fn max_frame_length(&self) -> usize { self.max_frame_length }
}

#[tokio::test]
async fn custom_codec_round_trips_frames() {
    let app = WireframeApp::<BincodeSerializer, (), Envelope>::new()
        .expect("build app")
        .with_codec(TaggedFrameCodec::new(64))
        .route(1, Arc::new(|_: &Envelope| Box::pin(async {})))
        .expect("route configured");

    let (mut client, server) = tokio::io::duplex(256);
    let server_task = tokio::spawn(async move {
        app.handle_connection_result(server)
            .await
            .expect("server should exit cleanly");
    });

    let request = Envelope::new(1, None, b"ping".to_vec());
    let payload = BincodeSerializer
        .serialize(&request)
        .expect("serialise request");

    let mut encoder = TaggedAdapter::new(64);
    let mut buf = BytesMut::new();
    encoder
        .encode(TaggedFrame { tag: 7, payload }, &mut buf)
        .expect("encode request");

    client.write_all(&buf).await.expect("write request");
    client.shutdown().await.expect("shutdown client");

    let mut output = Vec::new();
    client
        .read_to_end(&mut output)
        .await
        .expect("read response");

    server_task.await.expect("join server task");

    let mut decoder = TaggedAdapter::new(64);
    let mut response_buf = BytesMut::from(&output[..]);
    let response_frame = decoder
        .decode(&mut response_buf)
        .expect("decode response")
        .expect("response frame");
    assert!(response_buf.is_empty(), "unexpected trailing bytes");

    let (response_env, _) = BincodeSerializer
        .deserialize::<Envelope>(&response_frame.payload)
        .expect("deserialise response");
    assert_eq!(response_env.correlation_id(), Some(7));
    let response_payload = response_env.into_parts().payload();
    assert_eq!(response_payload, b"ping".to_vec());
}
