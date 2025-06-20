//! Tests covering response serialization and framing logic.
//!
//! These verify normal encoding as well as error conditions like
//! write failures and encode errors.

use bytes::BytesMut;
use wireframe::{
    app::WireframeApp,
    frame::{FrameProcessor, LengthFormat, LengthPrefixedProcessor},
    message::Message,
    serializer::BincodeSerializer,
};

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
async fn send_response_encodes_and_frames() {
    let app = WireframeApp::new()
        .unwrap()
        .frame_processor(LengthPrefixedProcessor::default())
        .serializer(BincodeSerializer);

    let mut out = Vec::new();
    app.send_response(&mut out, &TestResp(7)).await.unwrap();

    let processor = LengthPrefixedProcessor::default();
    let mut buf = BytesMut::from(&out[..]);
    let frame = processor.decode(&mut buf).unwrap().unwrap();
    let (decoded, _) = TestResp::from_bytes(&frame).unwrap();
    assert_eq!(decoded, TestResp(7));
}

#[tokio::test]
async fn length_prefixed_decode_requires_complete_header() {
    let processor = LengthPrefixedProcessor::default();
    let mut buf = BytesMut::from(&[0x00, 0x00, 0x00][..]); // only 3 bytes
    assert!(processor.decode(&mut buf).unwrap().is_none());
    assert_eq!(buf.len(), 3); // nothing consumed
}

#[tokio::test]
async fn length_prefixed_decode_requires_full_frame() {
    let processor = LengthPrefixedProcessor::default();
    let mut buf = BytesMut::from(&[0x00, 0x00, 0x00, 0x05, 0x01, 0x02][..]);
    assert!(processor.decode(&mut buf).unwrap().is_none());
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

#[tokio::test]
async fn send_response_propagates_write_error() {
    let app = WireframeApp::new()
        .unwrap()
        .frame_processor(LengthPrefixedProcessor::default());

    let mut writer = FailingWriter;
    let err = app
        .send_response(&mut writer, &TestResp(3))
        .await
        .expect_err("expected error");
    assert!(matches!(err, wireframe::app::SendError::Io(_)));
}

#[tokio::test]
async fn send_response_returns_encode_error() {
    let app = WireframeApp::new().unwrap();
    let err = app
        .send_response(&mut Vec::new(), &FailingResp)
        .await
        .expect_err("expected error");
    assert!(matches!(err, wireframe::app::SendError::Serialize(_)));
}

#[test]
fn custom_two_byte_big_endian_roundtrip() {
    let fmt = LengthFormat::u16_be();
    let processor = LengthPrefixedProcessor::new(fmt);
    let frame = vec![1, 2, 3, 4];
    let mut buf = BytesMut::new();
    processor.encode(&frame, &mut buf).unwrap();
    let decoded = processor.decode(&mut buf).unwrap().unwrap();
    assert_eq!(decoded, frame);
}

#[test]
fn custom_four_byte_little_endian_roundtrip() {
    let fmt = LengthFormat::u32_le();
    let processor = LengthPrefixedProcessor::new(fmt);
    let frame = vec![9, 8, 7];
    let mut buf = BytesMut::new();
    processor.encode(&frame, &mut buf).unwrap();
    assert_eq!(&buf[..4], u32::try_from(frame.len()).unwrap().to_le_bytes());
    let decoded = processor.decode(&mut buf).unwrap().unwrap();
    assert_eq!(decoded, frame);
}
