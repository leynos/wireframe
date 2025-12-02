#![cfg(not(loom))]
//! Tests covering response serialization and framing logic.
//!
//! These verify normal encoding as well as error conditions like
//! write failures and encode errors.

use std::sync::Arc;

use bytes::BytesMut;
use rstest::rstest;
use tokio_util::codec::{Decoder, Encoder, LengthDelimitedCodec};
use wireframe::{
    Serializer,
    app::{Envelope, Packet, WireframeApp},
    frame::{Endianness, LengthFormat},
    message::Message,
    serializer::BincodeSerializer,
};
use wireframe_testing::{decode_frames, decode_frames_with_max, encode_frame, run_app};

mod common;
use common::TestApp;

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
async fn send_response_encodes_and_frames() {
    let app = TestApp::new().expect("failed to create app");

    let mut out = Vec::new();
    app.send_response(&mut out, &TestResp(7))
        .await
        .expect("send_response failed");

    let frames = decode_frames(out);
    assert_eq!(frames.len(), 1, "expected a single response frame");
    let frame = frames.first().expect("expected frame missing");
    let (decoded, _) = TestResp::from_bytes(frame).expect("deserialize failed");
    assert_eq!(decoded, TestResp(7));
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

/// Ensures `send_response` permits frames up to the configured buffer capacity,
/// exceeding the codec's default 8 MiB limit.
#[tokio::test]
async fn send_response_honours_buffer_capacity() {
    let app = TestApp::new()
        .expect("failed to create app")
        .buffer_capacity(LARGE_FRAME);

    let payload = vec![0_u8; 9 * 1024 * 1024];
    let large = Large(payload.clone());
    let mut out = Vec::new();

    app.send_response(&mut out, &large)
        .await
        .expect("send_response failed");

    let frames = decode_frames_with_max(out, LARGE_FRAME);
    assert_eq!(frames.len(), 1, "expected a single response frame");
    let (decoded, _) = Large::from_bytes(&frames[0]).expect("deserialize failed");
    assert_eq!(decoded.0.len(), payload.len());
}

/// Verifies inbound and outbound codecs respect the application's buffer
/// capacity by round-tripping a 9 MiB payload.
#[tokio::test]
async fn process_stream_honours_buffer_capacity() {
    let app = TestApp::new()
        .expect("failed to create app")
        .buffer_capacity(LARGE_FRAME)
        .route(1, Arc::new(|_: &Envelope| Box::pin(async {})))
        .expect("route registration failed");

    let payload = vec![0_u8; 9 * 1024 * 1024];
    let env = Envelope::new(1, None, payload.clone());
    let bytes = BincodeSerializer.serialize(&env).expect("serialize failed");

    let mut codec = app.length_codec();
    let frame = encode_frame(&mut codec, bytes);
    let out = run_app(app, vec![frame], Some(10 * 1024 * 1024))
        .await
        .expect("run_app failed");

    let frames = decode_frames_with_max(out, LARGE_FRAME);
    assert_eq!(frames.len(), 1, "expected a single response frame");
    let (resp_env, _) = BincodeSerializer
        .deserialize::<Envelope>(&frames[0])
        .expect("deserialize failed");
    let resp_len = resp_env.into_parts().payload().len();
    assert_eq!(resp_len, payload.len());
}
