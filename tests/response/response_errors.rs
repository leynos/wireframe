//! Error-path tests and fixtures for response sending.

use bytes::{Bytes, BytesMut};
use tokio::io::{AsyncRead, ReadBuf};
use tokio_util::codec::{Decoder, Encoder, Framed};
use wireframe::{
    app::{Envelope, WireframeApp},
    codec::FrameCodec,
    serializer::BincodeSerializer,
};
use wireframe_testing::TestApp;

use super::TestResp;

#[derive(Debug)]
struct FailingResp;

impl bincode::Encode for FailingResp {
    fn encode<E: bincode::enc::Encoder>(
        &self,
        _: &mut E,
    ) -> Result<(), bincode::error::EncodeError> {
        Err(bincode::error::EncodeError::Other("fail"))
    }
}

impl<'de> bincode::BorrowDecode<'de, ()> for FailingResp {
    fn borrow_decode<D: bincode::de::BorrowDecoder<'de, Context = ()>>(
        _: &mut D,
    ) -> Result<Self, bincode::error::DecodeError> {
        Ok(FailingResp)
    }
}

struct FailingWriter;

impl tokio::io::AsyncWrite for FailingWriter {
    fn poll_write(
        self: std::pin::Pin<&mut Self>,
        _: &mut std::task::Context<'_>,
        _: &[u8],
    ) -> std::task::Poll<std::io::Result<usize>> {
        std::task::Poll::Ready(Err(std::io::Error::other("fail")))
    }

    fn poll_flush(
        self: std::pin::Pin<&mut Self>,
        _: &mut std::task::Context<'_>,
    ) -> std::task::Poll<std::io::Result<()>> {
        std::task::Poll::Ready(Ok(()))
    }

    fn poll_shutdown(
        self: std::pin::Pin<&mut Self>,
        _: &mut std::task::Context<'_>,
    ) -> std::task::Poll<std::io::Result<()>> {
        std::task::Poll::Ready(Ok(()))
    }
}

impl AsyncRead for FailingWriter {
    fn poll_read(
        self: std::pin::Pin<&mut Self>,
        _: &mut std::task::Context<'_>,
        _: &mut ReadBuf<'_>,
    ) -> std::task::Poll<std::io::Result<()>> {
        std::task::Poll::Ready(Ok(()))
    }
}

struct FailingFlushWriter {
    bytes: Vec<u8>,
}

impl tokio::io::AsyncWrite for FailingFlushWriter {
    fn poll_write(
        mut self: std::pin::Pin<&mut Self>,
        _: &mut std::task::Context<'_>,
        buf: &[u8],
    ) -> std::task::Poll<std::io::Result<usize>> {
        self.bytes.extend_from_slice(buf);
        std::task::Poll::Ready(Ok(buf.len()))
    }

    fn poll_flush(
        self: std::pin::Pin<&mut Self>,
        _: &mut std::task::Context<'_>,
    ) -> std::task::Poll<std::io::Result<()>> {
        std::task::Poll::Ready(Err(std::io::Error::other("flush failed")))
    }

    fn poll_shutdown(
        self: std::pin::Pin<&mut Self>,
        _: &mut std::task::Context<'_>,
    ) -> std::task::Poll<std::io::Result<()>> {
        std::task::Poll::Ready(Ok(()))
    }
}

#[derive(Clone, Debug)]
struct FailingEncodeFrameCodec;

#[derive(Debug)]
struct FailingEncodeFrame(Bytes);

struct FailingFrameDecoder;

impl Decoder for FailingFrameDecoder {
    type Error = std::io::Error;
    type Item = FailingEncodeFrame;

    fn decode(&mut self, _src: &mut BytesMut) -> std::io::Result<Option<Self::Item>> { Ok(None) }
}

struct FailingFrameEncoder;

impl Encoder<FailingEncodeFrame> for FailingFrameEncoder {
    type Error = std::io::Error;

    fn encode(&mut self, item: FailingEncodeFrame, _dst: &mut BytesMut) -> std::io::Result<()> {
        let _payload = item.0;
        Err(std::io::Error::other("frame encode failed"))
    }
}

impl FrameCodec for FailingEncodeFrameCodec {
    type Decoder = FailingFrameDecoder;
    type Encoder = FailingFrameEncoder;
    type Frame = FailingEncodeFrame;

    fn decoder(&self) -> Self::Decoder { FailingFrameDecoder }

    fn encoder(&self) -> Self::Encoder { FailingFrameEncoder }

    fn frame_payload(frame: &Self::Frame) -> &[u8] { frame.0.as_ref() }

    fn frame_payload_bytes(frame: &Self::Frame) -> Bytes { frame.0.clone() }

    fn wrap_payload(&self, payload: Bytes) -> Self::Frame { FailingEncodeFrame(payload) }

    fn max_frame_length(&self) -> usize { 1024 }
}

fn basic_app() -> wireframe::app::Result<WireframeApp<BincodeSerializer, (), Envelope>> {
    WireframeApp::<BincodeSerializer, (), Envelope>::new()
}

fn assert_io_error(err: &wireframe::app::SendError) {
    assert!(
        matches!(err, wireframe::app::SendError::Io(_)),
        "expected SendError::Io, got {err:?}"
    );
}

fn assert_serialize_error(err: &wireframe::app::SendError) {
    assert!(
        matches!(err, wireframe::app::SendError::Serialize(_)),
        "expected SendError::Serialize, got {err:?}"
    );
}

#[tokio::test]
async fn send_response_propagates_write_error() {
    let app = TestApp::new().expect("app creation failed");

    let mut writer = FailingWriter;
    let err = app
        .send_response(&mut writer, &TestResp(3))
        .await
        .expect_err("send_response should propagate write error");
    assert_io_error(&err);
}

#[tokio::test]
async fn send_response_propagates_frame_encoding_error() {
    let app = basic_app()
        .expect("failed to create app")
        .with_codec(FailingEncodeFrameCodec);
    let mut writer = Vec::new();

    let err = app
        .send_response(&mut writer, &TestResp(5))
        .await
        .expect_err("send_response should propagate codec encode failure");

    assert_io_error(&err);
}

#[tokio::test]
async fn send_response_propagates_flush_error_after_write() {
    let app = TestApp::new().expect("app creation failed");
    let mut writer = FailingFlushWriter { bytes: Vec::new() };

    let err = app
        .send_response(&mut writer, &TestResp(19))
        .await
        .expect_err("send_response should propagate flush failure");

    assert_io_error(&err);
    assert!(!writer.bytes.is_empty(), "response bytes should be written");
}

#[tokio::test]
async fn send_response_returns_encode_error() {
    let app = basic_app().expect("failed to create app");
    let err = app
        .send_response(&mut Vec::new(), &FailingResp)
        .await
        .expect_err("send_response should fail when encode errors");
    assert_serialize_error(&err);
}

#[tokio::test]
async fn send_response_framed_with_codec_propagates_serialization_error() {
    let app = basic_app().expect("failed to create app");
    let (client, _server) = tokio::io::duplex(1024);
    let mut framed = Framed::new(client, app.length_codec());

    let err = app
        .send_response_framed_with_codec(&mut framed, &FailingResp)
        .await
        .expect_err("framed send should fail before transport");

    assert_serialize_error(&err);
}

#[tokio::test]
async fn send_response_framed_with_codec_propagates_frame_encoding_error() {
    let app = basic_app()
        .expect("failed to create app")
        .with_codec(FailingEncodeFrameCodec);
    let (client, _server) = tokio::io::duplex(1024);
    let mut framed = Framed::new(client, FailingFrameEncoder);

    let err = app
        .send_response_framed_with_codec(&mut framed, &TestResp(13))
        .await
        .expect_err("framed send should propagate codec encode failure");

    assert_io_error(&err);
}

#[tokio::test]
async fn send_response_framed_propagates_serialization_error() {
    let app = basic_app().expect("failed to create app");
    let (client, _server) = tokio::io::duplex(1024);
    let mut framed = Framed::new(client, app.length_codec());

    let err = app
        .send_response_framed(&mut framed, &FailingResp)
        .await
        .expect_err("length-delimited send should fail before transport");

    assert_serialize_error(&err);
}

#[tokio::test]
async fn send_response_framed_propagates_io_error() {
    let app = basic_app().expect("failed to create app");
    let mut framed = Framed::new(FailingWriter, app.length_codec());

    let err = app
        .send_response_framed(&mut framed, &TestResp(17))
        .await
        .expect_err("length-delimited send should propagate write failure");

    assert_io_error(&err);
}
