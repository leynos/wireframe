use std::sync::{
    Arc,
    atomic::{AtomicUsize, Ordering},
};

use bytes::BytesMut;
use rstest::rstest;
use wireframe::{
    Serializer,
    app::WireframeApp,
    frame::FrameProcessor,
    message::Message,
    serializer::BincodeSerializer,
};

mod util;
use util::{default_processor, run_app_with_frame};

#[derive(bincode::Encode, bincode::BorrowDecode, PartialEq, Debug)]
struct TestEnvelope {
    id: u32,
    msg: Vec<u8>,
}

#[derive(bincode::Encode, bincode::BorrowDecode, PartialEq, Debug)]
struct Echo(u8);

#[rstest]
#[tokio::test]
async fn handler_receives_message_and_echoes_response(processor: LengthPrefixedProcessor) {
    let called = Arc::new(AtomicUsize::new(0));
    let called_clone = called.clone();
    let processor = default_processor();
    let app = WireframeApp::new()
        .unwrap()
        .frame_processor(processor.clone())
        .route(
            1,
            Box::new(move |_| {
                let called_inner = called_clone.clone();
                Box::pin(async move {
                    called_inner.fetch_add(1, Ordering::SeqCst);
                    // `WireframeApp` sends the envelope back automatically
                })
            }),
        )
        .unwrap();
    let msg_bytes = Echo(42).to_bytes().unwrap();
    let env = TestEnvelope {
        id: 1,
        msg: msg_bytes,
    };
    let env_bytes = BincodeSerializer.serialize(&env).unwrap();
    let mut framed = BytesMut::new();
    processor.encode(&env_bytes, &mut framed).unwrap();

    let out = run_app_with_frame(app, framed.to_vec()).await.unwrap();

    let mut buf = BytesMut::from(&out[..]);
    let frame = processor.decode(&mut buf).unwrap().unwrap();
    let (resp_env, _) = BincodeSerializer
        .deserialize::<TestEnvelope>(&frame)
        .unwrap();
    let (echo, _) = Echo::from_bytes(&resp_env.msg).unwrap();
    assert_eq!(echo, Echo(42));
    assert_eq!(called.load(Ordering::SeqCst), 1);
}
