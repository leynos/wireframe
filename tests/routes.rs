//! Tests for routing behaviour in `WireframeApp`.
//!
//! They validate handler invocation, echo responses, and sequential processing.

use std::sync::{
    Arc,
    atomic::{AtomicUsize, Ordering},
};

use bytes::BytesMut;
use rstest::rstest;
use tokio_util::codec::{Decoder, Encoder, LengthDelimitedCodec};
use wireframe::{
    Serializer,
    app::{Packet, PacketParts},
    message::Message,
    serializer::BincodeSerializer,
};
use wireframe_testing::{drive_with_bincode, drive_with_frames};

type TestApp = wireframe::app::WireframeApp<BincodeSerializer, (), TestEnvelope>;

#[derive(bincode::Encode, bincode::BorrowDecode, PartialEq, Debug, Clone)]
struct TestEnvelope {
    id: u32,
    correlation_id: Option<u64>,
    payload: Vec<u8>,
}

impl Packet for TestEnvelope {
    #[inline]
    fn id(&self) -> u32 { self.id }

    #[inline]
    fn correlation_id(&self) -> Option<u64> { self.correlation_id }

    fn into_parts(self) -> PacketParts {
        PacketParts::new(self.id, self.correlation_id, self.payload)
    }

    fn from_parts(parts: PacketParts) -> Self {
        let id = parts.id();
        let correlation_id = parts.correlation_id();
        let payload = parts.payload();
        Self {
            id,
            correlation_id,
            payload,
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
    let app = TestApp::new()
        .expect("failed to create app")
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
        correlation_id: Some(99),
        payload: msg_bytes,
    };

    let out = drive_with_bincode(app, env)
        .await
        .expect("drive_with_bincode failed");

    let mut codec = LengthDelimitedCodec::builder().new_codec();
    let mut buf = BytesMut::from(&out[..]);
    let frame = codec
        .decode(&mut buf)
        .expect("decode failed")
        .expect("frame missing");
    let (resp_env, _) = BincodeSerializer
        .deserialize::<TestEnvelope>(&frame)
        .expect("deserialize failed");
    assert_eq!(resp_env.correlation_id, Some(99));
    let (echo, _) = Echo::from_bytes(&resp_env.payload).expect("decode echo failed");
    assert_eq!(echo, Echo(42));
    assert_eq!(called.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn handler_echoes_with_none_correlation_id() {
    let app = TestApp::new()
        .expect("failed to create app")
        .route(
            1,
            std::sync::Arc::new(|_: &TestEnvelope| Box::pin(async {})),
        )
        .expect("route registration failed");

    let msg_bytes = Echo(7).to_bytes().expect("encode failed");
    let env = TestEnvelope {
        id: 1,
        correlation_id: None,
        payload: msg_bytes,
    };

    let out = drive_with_bincode(app, env).await.expect("drive failed");
    let mut codec = LengthDelimitedCodec::builder().new_codec();
    let mut buf = BytesMut::from(&out[..]);
    let frame = codec
        .decode(&mut buf)
        .expect("decode failed")
        .expect("missing frame");
    let (resp_env, _) = BincodeSerializer
        .deserialize::<TestEnvelope>(&frame)
        .expect("deserialize failed");

    assert_eq!(resp_env.correlation_id, None);
    let (echo, _) = Echo::from_bytes(&resp_env.payload).expect("decode echo failed");
    assert_eq!(echo, Echo(7));
}

#[tokio::test]
async fn multiple_frames_processed_in_sequence() {
    let app = TestApp::new()
        .expect("failed to create app")
        .route(
            1,
            std::sync::Arc::new(|_: &TestEnvelope| Box::pin(async {})),
        )
        .expect("route registration failed");

    let mut codec = LengthDelimitedCodec::builder().new_codec();
    let mut encoded_frames = Vec::new();
    for id in 1u8..=2 {
        let msg_bytes = Echo(id).to_bytes().expect("encode failed");
        let env = TestEnvelope {
            id: 1,
            correlation_id: Some(u64::from(id)),
            payload: msg_bytes,
        };
        let env_bytes = BincodeSerializer
            .serialize(&env)
            .expect("serialization failed");
        let mut framed = BytesMut::new();
        codec
            .encode(env_bytes.into(), &mut framed)
            .expect("encode failed");
        encoded_frames.push(framed.to_vec());
    }

    let out = drive_with_frames(app, encoded_frames)
        .await
        .expect("drive_with_frames failed");

    let mut buf = BytesMut::from(&out[..]);
    let first = codec
        .decode(&mut buf)
        .expect("decode failed")
        .expect("frame missing");
    let (env1, _) = BincodeSerializer
        .deserialize::<TestEnvelope>(&first)
        .expect("deserialize failed");
    let (echo1, _) = Echo::from_bytes(&env1.payload).expect("decode echo failed");
    let second = codec
        .decode(&mut buf)
        .expect("decode failed")
        .expect("frame missing");
    let (env2, _) = BincodeSerializer
        .deserialize::<TestEnvelope>(&second)
        .expect("deserialize failed");
    let (echo2, _) = Echo::from_bytes(&env2.payload).expect("decode echo failed");
    assert_eq!(env1.correlation_id, Some(1));
    assert_eq!(env2.correlation_id, Some(2));
    assert_eq!(echo1, Echo(1));
    assert_eq!(echo2, Echo(2));
}

#[rstest]
#[case(None)]
#[case(Some(1))]
#[case(Some(2))]
#[tokio::test]
async fn single_frame_propagates_correlation_id(#[case] cid: Option<u64>) {
    let app = TestApp::new()
        .expect("failed to create app")
        .route(
            1,
            std::sync::Arc::new(|_: &TestEnvelope| Box::pin(async {})),
        )
        .expect("route registration failed");

    let msg_bytes = Echo(5).to_bytes().expect("encode failed");
    let env = TestEnvelope {
        id: 1,
        correlation_id: cid,
        payload: msg_bytes,
    };
    let env_bytes = BincodeSerializer.serialize(&env).expect("serialize failed");

    let mut framed = BytesMut::new();
    let mut codec = LengthDelimitedCodec::builder().new_codec();
    codec
        .encode(env_bytes.into(), &mut framed)
        .expect("encode failed");

    let out = drive_with_frames(app, vec![framed.to_vec()])
        .await
        .expect("drive failed");
    let mut buf = BytesMut::from(&out[..]);
    let frame = codec
        .decode(&mut buf)
        .expect("decode failed")
        .expect("missing");
    let (resp, _) = BincodeSerializer
        .deserialize::<TestEnvelope>(&frame)
        .expect("deserialize failed");

    assert_eq!(resp.correlation_id, cid);
}

#[test]
fn packet_from_parts_round_trips() {
    let env = TestEnvelope {
        id: 5,
        correlation_id: Some(9),
        payload: vec![1, 2, 3],
    };
    let parts = env.clone().into_parts();
    let rebuilt = TestEnvelope::from_parts(parts);
    assert_eq!(rebuilt, env);
}
