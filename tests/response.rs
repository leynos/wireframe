//! Tests covering response serialization and framing logic.
//!
//! These verify normal encoding as well as error conditions like
//! write failures and encode errors.
#![cfg(not(loom))]

use std::sync::Arc;

use bytes::{Bytes, BytesMut};
use googletest::prelude::*;
use pretty_assertions::assert_eq;
use rstest::rstest;
use tokio::io::{AsyncRead, AsyncReadExt, ReadBuf};
use tokio_util::codec::{Decoder, Encoder, Framed, LengthDelimitedCodec};
use wireframe::{
    app::{Envelope, Packet, WireframeApp},
    codec::FrameCodec,
    frame::{Endianness, LengthFormat},
    message::Message,
    serializer::{BincodeSerializer, Serializer},
};
use wireframe_testing::{
    TestApp,
    TestResult,
    decode_frames,
    decode_frames_with_max,
    encode_frame,
    run_app,
};

// Larger cap used for oversized frame tests.
const LARGE_FRAME: usize = 16 * 1024 * 1024;

#[derive(bincode::Encode, bincode::BorrowDecode, PartialEq, Debug)]
struct TestResp(u32);

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

#[derive(bincode::Encode, bincode::BorrowDecode, PartialEq, Debug)]
struct Large(Vec<u8>);

/// Tests that sending a response serializes and frames the data correctly,
/// and that the response can be decoded and deserialized back to its original value asynchronously.
#[tokio::test]
async fn send_response_encodes_and_frames() -> TestResult {
    let app = TestApp::new().expect("failed to create app");

    let mut out = Vec::new();
    app.send_response(&mut out, &TestResp(7))
        .await
        .map_err(|e| format!("send_response failed: {e}"))?;

    let frames = decode_frames(out)?;
    assert_eq!(frames.len(), 1, "expected a single response frame");
    let frame = frames.first().ok_or("expected frame missing")?;
    let (decoded, _) =
        TestResp::from_bytes(frame).map_err(|e| format!("deserialize failed: {e}"))?;
    assert_eq!(decoded, TestResp(7), "decoded payload mismatch");
    Ok(())
}

/// Tests that decoding with an incomplete length prefix header returns `None` and does not consume
/// any bytes from the buffer.
///
/// This ensures that the decoder waits for the full header before attempting to decode a frame.
#[tokio::test]
async fn length_prefixed_decode_requires_complete_header() {
    let app = TestApp::new().expect("failed to create app");
    let mut codec = app.length_codec();
    let mut buf = BytesMut::from(&[0x00, 0x00, 0x00][..]); // only 3 bytes
    assert!(codec.decode(&mut buf).expect("decode failed").is_none());
    assert_eq!(buf.len(), 3); // nothing consumed
}

/// Tests that decoding with a complete length prefix but incomplete frame data returns `None`
/// and consumes only the 4-byte length prefix.
///
/// Confirms that the decoder leaves the incomplete body in the buffer until the full frame arrives.
#[tokio::test]
async fn length_prefixed_decode_requires_full_frame() {
    let app = TestApp::new().expect("failed to create app");
    let mut codec = app.length_codec();
    let mut buf = BytesMut::from(&[0x00, 0x00, 0x00, 0x05, 0x01, 0x02][..]);
    assert!(codec.decode(&mut buf).expect("decode failed").is_none());
    // LengthDelimitedCodec consumes the length prefix even if the frame
    // remains incomplete.
    assert_eq!(buf.len(), 2);
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

#[rstest]
#[case(LengthFormat::u16_be(), vec![1, 2, 3, 4], vec![0x00, 0x04])]
#[case(LengthFormat::u32_le(), vec![9, 8, 7], vec![3, 0, 0, 0])]
fn custom_length_roundtrip(
    #[case] fmt: LengthFormat,
    #[case] frame: Vec<u8>,
    #[case] prefix: Vec<u8>,
) {
    let mut builder = LengthDelimitedCodec::builder();
    builder.length_field_length(fmt.bytes());
    if fmt.endianness() == Endianness::Little {
        builder.little_endian();
    }
    let mut codec = builder.new_codec();
    let mut buf = BytesMut::with_capacity(frame.len() + prefix.len());
    codec
        .encode(frame.clone().into(), &mut buf)
        .expect("encode failed");
    let head = buf
        .get(..prefix.len())
        .expect("encoded buffer shorter than prefix");
    assert_eq!(head, &prefix[..]);
    let decoded = codec
        .decode(&mut buf)
        .expect("decode failed")
        .expect("frame missing");
    assert_eq!(decoded, frame);
    assert!(buf.is_empty(), "unexpected trailing bytes after decode");
}

#[tokio::test]
async fn send_response_propagates_write_error() {
    let app = TestApp::new().expect("app creation failed");

    let mut writer = FailingWriter;
    let err = app
        .send_response(&mut writer, &TestResp(3))
        .await
        .expect_err("send_response should propagate write error");
    assert!(matches!(err, wireframe::app::SendError::Io(_)));
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

    assert_that!(matches!(err, wireframe::app::SendError::Io(_)), eq(true));
    assert!(!writer.bytes.is_empty(), "response bytes should be written");
}

#[rstest]
#[case(0, Endianness::Big)]
#[case(9, Endianness::Little)]
fn encode_fails_for_invalid_prefix_size(#[case] bytes: usize, #[case] endian: Endianness) {
    let err = LengthFormat::try_new(bytes, endian).expect_err("invalid width should error");
    assert_eq!(err.kind(), std::io::ErrorKind::InvalidInput);
}

#[rstest]
#[case(0, Endianness::Little)]
#[case(9, Endianness::Big)]
fn decode_fails_for_invalid_prefix_size(#[case] bytes: usize, #[case] endian: Endianness) {
    let err = LengthFormat::try_new(bytes, endian).expect_err("invalid width should error");
    assert_eq!(err.kind(), std::io::ErrorKind::InvalidInput);
}

#[rstest]
#[case(LengthFormat::new(1, Endianness::Big), 256)]
#[case(LengthFormat::new(2, Endianness::Little), 65_536)]
fn encode_fails_for_length_too_large(#[case] fmt: LengthFormat, #[case] len: usize) {
    let frame = vec![0u8; len];
    let mut buf = BytesMut::new();
    let err = fmt
        .write_len(frame.len(), &mut buf)
        .expect_err("expected error");
    assert_eq!(err.kind(), std::io::ErrorKind::InvalidInput);
}

/// Tests that `send_response` returns a serialization error when encoding fails.
///
/// This test sends a `FailingResp` using `send_response` and asserts that the resulting
/// error is of the `Serialize` variant, indicating a failure during response encoding.
#[tokio::test]
async fn send_response_returns_encode_error() {
    // Use a type that fails during serialization; encode should fail before any framing.
    let app = WireframeApp::<BincodeSerializer, (), Envelope>::new().expect("failed to create app");
    let err = app
        .send_response(&mut Vec::new(), &FailingResp)
        .await
        .expect_err("send_response should fail when encode errors");
    assert!(matches!(err, wireframe::app::SendError::Serialize(_)));
}

#[tokio::test]
async fn send_response_framed_with_codec_writes_encoded_frame() {
    let app = TestApp::new().expect("failed to create app");
    let (client, mut server) = tokio::io::duplex(1024);
    let mut framed = Framed::new(client, app.length_codec());

    app.send_response_framed_with_codec(&mut framed, &TestResp(11))
        .await
        .expect("framed send should succeed");
    drop(framed);

    let mut out = Vec::new();
    server
        .read_to_end(&mut out)
        .await
        .expect("read framed bytes");
    let decoded_frames = decode_frames(out).expect("decode response frames");
    assert_eq!(decoded_frames.len(), 1);
    let frame = decoded_frames.first().expect("response frame missing");
    let (decoded, _) = TestResp::from_bytes(frame).expect("deserialize response");
    assert_eq!(decoded, TestResp(11));
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

#[tokio::test]
async fn send_response_framed_sends_raw_serialized_payload() -> TestResult {
    let app = WireframeApp::<BincodeSerializer, (), Envelope>::new().expect("failed to create app");
    let (client, mut server) = tokio::io::duplex(1024);
    let mut framed = Framed::new(client, app.length_codec());
    let message = TestResp(23);
    let expected = BincodeSerializer
        .serialize(&message)
        .map_err(|e| format!("serialize failed: {e}"))?;

    app.send_response_framed(&mut framed, &message)
        .await
        .map_err(|e| format!("framed send failed: {e}"))?;
    drop(framed);

    let mut out = Vec::new();
    server
        .read_to_end(&mut out)
        .await
        .map_err(|e| format!("read framed output failed: {e}"))?;
    let decoded_frames = decode_frames(out)?;
    assert_eq!(decoded_frames.len(), 1);
    let frame = decoded_frames.first().ok_or("response frame missing")?;
    assert_eq!(frame, &expected);
    Ok(())
}

#[tokio::test]
async fn send_response_framed_honours_buffer_capacity() -> TestResult {
    let app = TestApp::new()?.buffer_capacity(LARGE_FRAME);
    let (client, mut server) = tokio::io::duplex(10 * 1024 * 1024);
    let mut framed = Framed::new(client, app.length_codec());
    let payload = vec![0_u8; 9 * 1024 * 1024];

    app.send_response_framed(&mut framed, &Large(payload.clone()))
        .await
        .map_err(|e| format!("framed send failed: {e}"))?;
    drop(framed);

    let mut out = Vec::new();
    server
        .read_to_end(&mut out)
        .await
        .map_err(|e| format!("read framed output failed: {e}"))?;
    let decoded_frames = decode_frames_with_max(out, LARGE_FRAME)?;
    assert_eq!(decoded_frames.len(), 1);
    let frame = decoded_frames.first().ok_or("response frame missing")?;
    let (decoded, _) = Large::from_bytes(frame).map_err(|e| format!("deserialize failed: {e}"))?;
    assert_eq!(decoded.0.len(), payload.len());
    Ok(())
}

/// Ensures `send_response` permits frames up to the configured buffer capacity,
/// exceeding the codec's default 8 MiB limit.
#[tokio::test]
async fn send_response_honours_buffer_capacity() -> TestResult {
    let app = TestApp::new()?.buffer_capacity(LARGE_FRAME);

    let payload = vec![0_u8; 9 * 1024 * 1024];
    let large = Large(payload.clone());
    let mut out = Vec::new();

    app.send_response(&mut out, &large)
        .await
        .map_err(|e| format!("send_response failed: {e}"))?;

    let frames = decode_frames_with_max(out, LARGE_FRAME)?;
    assert_eq!(frames.len(), 1, "expected a single response frame");
    let frame = frames.first().ok_or("response frame missing")?;
    let (decoded, _) = Large::from_bytes(frame).map_err(|e| format!("deserialize failed: {e}"))?;
    assert_eq!(decoded.0.len(), payload.len());
    Ok(())
}

/// Verifies inbound and outbound codecs respect the application's buffer
/// capacity by round-tripping a 9 MiB payload.
#[tokio::test]
async fn process_stream_honours_buffer_capacity() -> TestResult {
    let app = TestApp::new()?
        .buffer_capacity(LARGE_FRAME)
        .route(1, Arc::new(|_: &Envelope| Box::pin(async {})))?;

    let payload = vec![0_u8; 9 * 1024 * 1024];
    let env = Envelope::new(1, None, payload.clone());
    let bytes = BincodeSerializer
        .serialize(&env)
        .map_err(|e| format!("serialize failed: {e}"))?;

    let mut codec = app.length_codec();
    let frame = encode_frame(&mut codec, bytes)?;
    let out = run_app(app, vec![frame], Some(10 * 1024 * 1024)).await?;

    let frames = decode_frames_with_max(out, LARGE_FRAME)?;
    assert_eq!(frames.len(), 1, "expected a single response frame");
    let frame = frames.first().ok_or("response frame missing")?;
    let (resp_env, _) = BincodeSerializer
        .deserialize::<Envelope>(frame)
        .map_err(|e| format!("deserialize failed: {e}"))?;
    let resp_len = resp_env.into_parts().into_payload().len();
    assert_eq!(resp_len, payload.len());
    Ok(())
}
