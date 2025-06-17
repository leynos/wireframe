use bytes::BytesMut;
use wireframe::{
    app::WireframeApp,
    config::SerializationFormat,
    frame::{FrameProcessor, LengthPrefixedProcessor},
    message::Message,
};

#[derive(bincode::Encode, bincode::BorrowDecode, PartialEq, Debug)]
struct TestResp(u32);

#[tokio::test]
async fn send_response_encodes_and_frames() {
    let mut app = WireframeApp::new()
        .unwrap()
        .frame_processor(LengthPrefixedProcessor)
        .unwrap()
        .serialization_format(SerializationFormat::Bincode)
        .unwrap();

    let mut out = Vec::new();
    app.send_response(&mut out, &TestResp(7)).await.unwrap();

    let mut processor = LengthPrefixedProcessor;
    let mut buf = BytesMut::from(&out[..]);
    let frame = processor.decode(&mut buf).await.unwrap().unwrap();
    let (decoded, _) = TestResp::from_bytes(&frame).unwrap();
    assert_eq!(decoded, TestResp(7));
}

#[tokio::test]
async fn length_prefixed_decode_requires_complete_header() {
    let mut processor = LengthPrefixedProcessor;
    let mut buf = BytesMut::from(&[0x00, 0x00, 0x00][..]); // only 3 bytes
    assert!(processor.decode(&mut buf).await.unwrap().is_none());
    assert_eq!(buf.len(), 3); // nothing consumed
}

#[tokio::test]
async fn length_prefixed_decode_requires_full_frame() {
    let mut processor = LengthPrefixedProcessor;
    let mut buf = BytesMut::from(&[0x00, 0x00, 0x00, 0x05, 0x01, 0x02][..]);
    assert!(processor.decode(&mut buf).await.unwrap().is_none());
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
    let mut app = WireframeApp::new()
        .unwrap()
        .frame_processor(LengthPrefixedProcessor)
        .unwrap();

    let mut writer = FailingWriter;
    let err = app
        .send_response(&mut writer, &TestResp(3))
        .await
        .expect_err("expected error");
    assert_eq!(err.kind(), std::io::ErrorKind::Other);
}
