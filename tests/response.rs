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
