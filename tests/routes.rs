//! Tests for routing behaviour in `WireframeApp`.
//!
//! They validate handler invocation, echo responses, and sequential processing.

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
use wireframe_testing::{drive_with_bincode, drive_with_frames};

#[derive(bincode::Encode, bincode::BorrowDecode, PartialEq, Debug)]
struct TestEnvelope {
    id: u32,
    correlation_id: u64,
    msg: Vec<u8>,
}

impl wireframe::app::Packet for TestEnvelope {
    fn id(&self) -> u32 { self.id }

    fn correlation_id(&self) -> u64 { self.correlation_id }

    fn into_parts(self) -> (u32, u64, Vec<u8>) { (self.id, self.correlation_id, self.msg) }

    fn from_parts(id: u32, correlation_id: u64, msg: Vec<u8>) -> Self {
        Self {
            id,
            correlation_id,
            msg,
        }
    }
}

#[derive(bincode::Encode, bincode::BorrowDecode, PartialEq, Debug)]
struct Echo(u8);

#[rstest]
#[tokio::test]
async fn handler_receives_message_and_echoes_response() {
    let called = Arc::new(AtomicUsize::new(0));
    let called_clone = called.clone();
    let app = WireframeApp::<_, _, TestEnvelope>::new()
        .expect("failed to create app")
        .frame_processor(LengthPrefixedProcessor::default())
        .route(
            1,
            std::sync::Arc::new(move |_: &TestEnvelope| {
                let called_inner = called_clone.clone();
                Box::pin(async move {
                    called_inner.fetch_add(1, Ordering::SeqCst);
                    // `WireframeApp` sends the envelope back automatically
                })
            }),
        )
        .expect("route registration failed");
    let msg_bytes = Echo(42).to_bytes().expect("encode failed");
    let env = TestEnvelope {
        id: 1,
        correlation_id: 99,
        msg: msg_bytes,
    };

    let out = drive_with_bincode(app, env)
        .await
        .expect("drive_with_bincode failed");

    let mut buf = BytesMut::from(&out[..]);
    let frame = LengthPrefixedProcessor::default()
        .decode(&mut buf)
        .expect("decode failed")
        .expect("frame missing");
    let (resp_env, _) = BincodeSerializer
        .deserialize::<TestEnvelope>(&frame)
        .expect("deserialize failed");
    assert_eq!(resp_env.correlation_id, 99);
    let (echo, _) = Echo::from_bytes(&resp_env.msg).expect("decode echo failed");
    assert_eq!(echo, Echo(42));
    assert_eq!(called.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn multiple_frames_processed_in_sequence() {
    let app = WireframeApp::<_, _, TestEnvelope>::new()
        .expect("failed to create app")
        .frame_processor(LengthPrefixedProcessor::default())
        .route(
            1,
            std::sync::Arc::new(|_: &TestEnvelope| Box::pin(async {})),
        )
        .expect("route registration failed");

    let frames: Vec<Vec<u8>> = (1u8..=2)
        .map(|id| {
            let msg_bytes = Echo(id).to_bytes().expect("encode failed");
            let env = TestEnvelope {
                id: 1,
                correlation_id: u64::from(id),
                msg: msg_bytes,
            };
            let env_bytes = BincodeSerializer
                .serialize(&env)
                .expect("serialization failed");
            let mut framed = BytesMut::new();
            LengthPrefixedProcessor::default()
                .encode(&env_bytes, &mut framed)
                .expect("encode failed");
            framed.to_vec()
        })
        .collect();

    let out = drive_with_frames(app, frames)
        .await
        .expect("drive_with_frames failed");

    let mut buf = BytesMut::from(&out[..]);
    let first = LengthPrefixedProcessor::default()
        .decode(&mut buf)
        .expect("decode failed")
        .expect("frame missing");
    let (env1, _) = BincodeSerializer
        .deserialize::<TestEnvelope>(&first)
        .expect("deserialize failed");
    let (echo1, _) = Echo::from_bytes(&env1.msg).expect("decode echo failed");
    let second = LengthPrefixedProcessor::default()
        .decode(&mut buf)
        .expect("decode failed")
        .expect("frame missing");
    let (env2, _) = BincodeSerializer
        .deserialize::<TestEnvelope>(&second)
        .expect("deserialize failed");
    let (echo2, _) = Echo::from_bytes(&env2.msg).expect("decode echo failed");
    assert_eq!(env1.correlation_id, 1);
    assert_eq!(env2.correlation_id, 2);
    assert_eq!(echo1, Echo(1));
    assert_eq!(echo2, Echo(2));
}
