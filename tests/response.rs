//! Tests covering response serialization and framing logic.
//!
//! These verify normal encoding as well as error conditions like
//! write failures and encode errors.

use bytes::BytesMut;
use rstest::rstest;
use wireframe::{
    app::Envelope,
    frame::{Endianness, FrameProcessor, LengthFormat, LengthPrefixedProcessor},
    message::Message,
    serializer::BincodeSerializer,
};

type TestApp<S = BincodeSerializer, C = (), E = Envelope> = wireframe::app::WireframeApp<S, C, E>;

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

#[tokio::test]
/// Tests that sending a response serialises and frames the data correctly,
/// and that the response can be decoded and deserialised back to its original value asynchronously.
async fn send_response_encodes_and_frames() {
    let app = TestApp::<_, _, Envelope>::new()
        .expect("failed to create app")
        .serializer(BincodeSerializer);

    let mut out = Vec::new();
    app.send_response(&mut out, &TestResp(7))
        .await
        .expect("send_response failed");

    let processor = LengthPrefixedProcessor::default();
    let mut buf = BytesMut::from(&out[..]);
    let frame = processor
        .decode(&mut buf)
        .expect("decode failed")
        .expect("frame missing");
    let (decoded, _) = TestResp::from_bytes(&frame).expect("deserialize failed");
    assert_eq!(decoded, TestResp(7));
}

#[tokio::test]
/// Tests that decoding with an incomplete length prefix header returns `None` and does not consume
/// any bytes from the buffer.
///
/// This ensures that the decoder waits for the full header before attempting to decode a frame.
async fn length_prefixed_decode_requires_complete_header() {
    let processor = LengthPrefixedProcessor::default();
    let mut buf = BytesMut::from(&[0x00, 0x00, 0x00][..]); // only 3 bytes
    assert!(processor.decode(&mut buf).expect("decode failed").is_none());
    assert_eq!(buf.len(), 3); // nothing consumed
}

#[tokio::test]
/// Tests that decoding with a complete length prefix but incomplete frame data returns `None`
/// and retains all bytes in the buffer.
///
/// Ensures that the decoder does not consume any bytes when the full frame is not yet available.
async fn length_prefixed_decode_requires_full_frame() {
    let processor = LengthPrefixedProcessor::default();
    let mut buf = BytesMut::from(&[0x00, 0x00, 0x00, 0x05, 0x01, 0x02][..]);
    assert!(processor.decode(&mut buf).expect("decode failed").is_none());
    // buffer should retain bytes since frame isn't complete
    assert_eq!(buf.len(), 6);
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
    let processor = LengthPrefixedProcessor::new(fmt);
    let mut buf = BytesMut::new();
    processor.encode(&frame, &mut buf).expect("encode failed");
    assert_eq!(&buf[..prefix.len()], &prefix[..]);
    let decoded = processor
        .decode(&mut buf)
        .expect("decode failed")
        .expect("frame missing");
    assert_eq!(decoded, frame);
}

#[tokio::test]
async fn send_response_propagates_write_error() {
    let app = TestApp::<_, _, Envelope>::new()
        .expect("app creation failed");

    let mut writer = FailingWriter;
    let err = app
        .send_response(&mut writer, &TestResp(3))
        .await
        .expect_err("send_response should propagate write error");
    assert!(matches!(err, wireframe::app::SendError::Io(_)));
}

#[rstest]
#[case(0, Endianness::Big)]
#[case(3, Endianness::Big)]
#[case(5, Endianness::Little)]
fn encode_fails_for_invalid_prefix_size(#[case] bytes: usize, #[case] endian: Endianness) {
    let fmt = LengthFormat::new(bytes, endian);
    let processor = LengthPrefixedProcessor::new(fmt);
    let mut buf = BytesMut::new();
    let err = processor
        .encode(&vec![1, 2], &mut buf)
        .expect_err("encode must fail for unsupported prefix size");
    assert_eq!(err.kind(), std::io::ErrorKind::InvalidInput);
}

#[rstest]
#[case(0, Endianness::Little)]
#[case(3, Endianness::Little)]
#[case(5, Endianness::Big)]
fn decode_fails_for_invalid_prefix_size(#[case] bytes: usize, #[case] endian: Endianness) {
    let fmt = LengthFormat::new(bytes, endian);
    let processor = LengthPrefixedProcessor::new(fmt);
    let mut buf = BytesMut::from(vec![0u8; bytes].as_slice());
    let err = processor
        .decode(&mut buf)
        .expect_err("decode must fail for unsupported prefix size");
    assert_eq!(err.kind(), std::io::ErrorKind::InvalidInput);
}

#[rstest]
#[case(LengthFormat::new(1, Endianness::Big), 256)]
#[case(LengthFormat::new(2, Endianness::Little), 65_536)]
fn encode_fails_for_length_too_large(#[case] fmt: LengthFormat, #[case] len: usize) {
    let processor = LengthPrefixedProcessor::new(fmt);
    let frame = vec![0u8; len];
    let mut buf = BytesMut::new();
    let err = processor
        .encode(&frame, &mut buf)
        .expect_err("expected error");
    assert_eq!(err.kind(), std::io::ErrorKind::InvalidInput);
}

#[tokio::test]
/// Tests that `send_response` returns a serialization error when encoding fails.
///
/// This test sends a `FailingResp` using `send_response` and asserts that the resulting
/// error is of the `Serialize` variant, indicating a failure during response encoding.
async fn send_response_returns_encode_error() {
    let app = TestApp::<_, _, Envelope>::new().expect("failed to create app");
    let err = app
        .send_response(&mut Vec::new(), &FailingResp)
        .await
        .expect_err("send_response should fail when encode errors");
    assert!(matches!(err, wireframe::app::SendError::Serialize(_)));
}
