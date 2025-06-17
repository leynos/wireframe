use bytes::BytesMut;
use wireframe::{
    app::WireframeApp,
    config::SerializationFormat,
    frame::{FrameProcessor, LengthPrefixedProcessor},
    message::Message,
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
        .frame_processor(LengthPrefixedProcessor)
        .serialization_format(SerializationFormat::Bincode);

    let mut out = Vec::new();
    app.send_response(&mut out, &TestResp(7)).await.unwrap();

    let processor = LengthPrefixedProcessor;
    let mut buf = BytesMut::from(&out[..]);
    let frame = processor.decode(&mut buf).unwrap().unwrap();
    let (decoded, _) = TestResp::from_bytes(&frame).unwrap();
    assert_eq!(decoded, TestResp(7));
}

#[tokio::test]
async fn length_prefixed_decode_requires_complete_header() {
    let processor = LengthPrefixedProcessor;
    let mut buf = BytesMut::from(&[0x00, 0x00, 0x00][..]); // only 3 bytes
    assert!(processor.decode(&mut buf).unwrap().is_none());
    assert_eq!(buf.len(), 3); // nothing consumed
}

#[tokio::test]
async fn length_prefixed_decode_requires_full_frame() {
    let processor = LengthPrefixedProcessor;
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
        .frame_processor(LengthPrefixedProcessor);

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
