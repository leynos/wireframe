use std::sync::{
    Arc,
    atomic::{AtomicUsize, Ordering},
};

use bytes::BytesMut;
use rstest::rstest;
use wireframe::{
    Serializer,
    app::WireframeApp,
    frame::{FrameProcessor, LengthPrefixedProcessor},
    message::Message,
    serializer::BincodeSerializer,
};

mod util;
use util::{run_app_with_frame, run_app_with_frames};

#[derive(bincode::Encode, bincode::BorrowDecode, PartialEq, Debug)]
struct TestEnvelope {
    id: u32,
    msg: Vec<u8>,
}

#[derive(bincode::Encode, bincode::BorrowDecode, PartialEq, Debug)]
struct Echo(u8);

#[rstest]
#[tokio::test]
async fn handler_receives_message_and_echoes_response() {
    let called = Arc::new(AtomicUsize::new(0));
    let called_clone = called.clone();
    let app = WireframeApp::new()
        .unwrap()
        .frame_processor(LengthPrefixedProcessor::default())
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
    LengthPrefixedProcessor::default()
        .encode(&env_bytes, &mut framed)
        .unwrap();

    let out = run_app_with_frame(app, framed.to_vec()).await.unwrap();

    let mut buf = BytesMut::from(&out[..]);
    let frame = LengthPrefixedProcessor::default()
        .decode(&mut buf)
        .unwrap()
        .unwrap();
    let (resp_env, _) = BincodeSerializer
        .deserialize::<TestEnvelope>(&frame)
        .unwrap();
    let (echo, _) = Echo::from_bytes(&resp_env.msg).unwrap();
    assert_eq!(echo, Echo(42));
    assert_eq!(called.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn multiple_frames_processed_in_sequence() {
    let app = WireframeApp::new()
        .unwrap()
        .frame_processor(LengthPrefixedProcessor::default())
        .route(1, Box::new(|_| Box::pin(async {})))
        .unwrap();

    let frames: Vec<Vec<u8>> = (1u8..=2)
        .map(|id| {
            let msg_bytes = Echo(id).to_bytes().unwrap();
            let env = TestEnvelope {
                id: 1,
                msg: msg_bytes,
            };
            let env_bytes = BincodeSerializer.serialize(&env).unwrap();
            let mut framed = BytesMut::new();
            LengthPrefixedProcessor::default()
                .encode(&env_bytes, &mut framed)
                .unwrap();
            framed.to_vec()
        })
        .collect();

    let out = run_app_with_frames(app, frames).await.unwrap();

    let mut buf = BytesMut::from(&out[..]);
    let first = LengthPrefixedProcessor::default()
        .decode(&mut buf)
        .unwrap()
        .unwrap();
    let (env1, _) = BincodeSerializer
        .deserialize::<TestEnvelope>(&first)
        .unwrap();
    let (echo1, _) = Echo::from_bytes(&env1.msg).unwrap();
    let second = LengthPrefixedProcessor::default()
        .decode(&mut buf)
        .unwrap()
        .unwrap();
    let (env2, _) = BincodeSerializer
        .deserialize::<TestEnvelope>(&second)
        .unwrap();
    let (echo2, _) = Echo::from_bytes(&env2.msg).unwrap();
    assert_eq!(echo1, Echo(1));
    assert_eq!(echo2, Echo(2));
}
