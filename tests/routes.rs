#![cfg(not(loom))]
//! Tests for routing behaviour in `WireframeApp`.
//!
//! They validate handler invocation, echo responses, and sequential processing.

mod common;

use std::sync::{
    Arc,
    atomic::{AtomicUsize, Ordering},
};

use bytes::BytesMut;
use common::TestResult;
use rstest::rstest;
use tokio_util::codec::Encoder;
use wireframe::{
    Serializer,
    app::{Packet, PacketParts},
    message::Message,
    serializer::BincodeSerializer,
};
use wireframe_testing::{
    TEST_MAX_FRAME,
    decode_frames,
    drive_with_bincode,
    drive_with_frames,
    new_test_codec,
};

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

#[tokio::test]
#[expect(
    clippy::panic_in_result_fn,
    reason = "asserts provide clearer diagnostics in tests"
)]
async fn handler_receives_message_and_echoes_response() -> TestResult<()> {
    let called = Arc::new(AtomicUsize::new(0));
    let called_clone = called.clone();
    let app = TestApp::new()?.route(
        1,
        std::sync::Arc::new(move |_: &TestEnvelope| {
            let called_inner = called_clone.clone();
            Box::pin(async move {
                called_inner.fetch_add(1, Ordering::SeqCst);
                // `WireframeApp` sends the envelope back automatically
            })
        }),
    )?;
    let msg_bytes = Echo(42).to_bytes()?;
    let env = TestEnvelope {
        id: 1,
        correlation_id: Some(99),
        payload: msg_bytes,
    };

    let out = drive_with_bincode(app, env).await?;

    let frames = decode_frames(out);
    let [first] = frames.as_slice() else {
        return Err("expected a single response frame".into());
    };
    let (resp_env, _) = BincodeSerializer.deserialize::<TestEnvelope>(first)?;
    assert_eq!(resp_env.correlation_id, Some(99), "correlation id mismatch");
    let (echo, _) = Echo::from_bytes(&resp_env.payload)?;
    assert_eq!(echo, Echo(42), "echo payload mismatch");
    assert_eq!(
        called.load(Ordering::SeqCst),
        1,
        "route not invoked exactly once"
    );
    Ok(())
}

#[tokio::test]
#[expect(
    clippy::panic_in_result_fn,
    reason = "asserts provide clearer diagnostics in tests"
)]
async fn handler_echoes_with_none_correlation_id() -> TestResult<()> {
    let app = TestApp::new()?.route(
        1,
        std::sync::Arc::new(|_: &TestEnvelope| Box::pin(async {})),
    )?;

    let msg_bytes = Echo(7).to_bytes()?;
    let env = TestEnvelope {
        id: 1,
        correlation_id: None,
        payload: msg_bytes,
    };

    let out = drive_with_bincode(app, env).await?;
    let frames = decode_frames(out);
    let [first] = frames.as_slice() else {
        return Err("expected a single response frame".into());
    };
    let (resp_env, _) = BincodeSerializer.deserialize::<TestEnvelope>(first)?;

    assert!(
        resp_env.correlation_id.is_none(),
        "unexpected correlation id"
    );
    let (echo, _) = Echo::from_bytes(&resp_env.payload)?;
    assert_eq!(echo, Echo(7), "echo payload mismatch");
    Ok(())
}

#[tokio::test]
#[expect(
    clippy::panic_in_result_fn,
    reason = "asserts provide clearer diagnostics in tests"
)]
async fn multiple_frames_processed_in_sequence() -> TestResult<()> {
    let app = TestApp::new()?.route(
        1,
        std::sync::Arc::new(|_: &TestEnvelope| Box::pin(async {})),
    )?;

    let mut codec = new_test_codec(TEST_MAX_FRAME);
    let mut encoded_frames = Vec::new();
    for id in 1u8..=2 {
        let msg_bytes = Echo(id).to_bytes()?;
        let env = TestEnvelope {
            id: 1,
            correlation_id: Some(u64::from(id)),
            payload: msg_bytes,
        };
        let env_bytes = BincodeSerializer.serialize(&env)?;
        let mut framed = BytesMut::with_capacity(env_bytes.len() + 4);
        codec.encode(env_bytes.into(), &mut framed)?;
        encoded_frames.push(framed.to_vec());
    }

    let out = drive_with_frames(app, encoded_frames).await?;

    let frames = decode_frames(out);
    let [first, second] = frames.as_slice() else {
        return Err("expected two response frames".into());
    };
    let (env1, _) = BincodeSerializer.deserialize::<TestEnvelope>(first)?;
    let (echo1, _) = Echo::from_bytes(&env1.payload)?;
    let (env2, _) = BincodeSerializer.deserialize::<TestEnvelope>(second)?;
    let (echo2, _) = Echo::from_bytes(&env2.payload)?;
    assert_eq!(
        env1.correlation_id,
        Some(1),
        "first correlation id mismatch"
    );
    assert_eq!(
        env2.correlation_id,
        Some(2),
        "second correlation id mismatch"
    );
    assert_eq!(echo1, Echo(1), "first echo payload mismatch");
    assert_eq!(echo2, Echo(2), "second echo payload mismatch");
    Ok(())
}

#[rstest]
#[case(None)]
#[case(Some(1))]
#[case(Some(2))]
#[tokio::test]
async fn single_frame_propagates_correlation_id(#[case] cid: Option<u64>) -> TestResult<()> {
    let app = TestApp::new()?.route(
        1,
        std::sync::Arc::new(|_: &TestEnvelope| Box::pin(async {})),
    )?;

    let msg_bytes = Echo(5).to_bytes()?;
    let env = TestEnvelope {
        id: 1,
        correlation_id: cid,
        payload: msg_bytes,
    };
    let env_bytes = BincodeSerializer.serialize(&env)?;

    let mut frame_buf = BytesMut::with_capacity(env_bytes.len() + 4);
    let mut codec = new_test_codec(TEST_MAX_FRAME);
    codec.encode(env_bytes.into(), &mut frame_buf)?;

    let out = drive_with_frames(app, vec![frame_buf.to_vec()]).await?;
    let frames = decode_frames(out);
    let [first] = frames.as_slice() else {
        return Err("expected a single response frame".into());
    };
    let (resp, _) = BincodeSerializer.deserialize::<TestEnvelope>(first)?;

    assert_eq!(resp.correlation_id, cid, "correlation id mismatch");
    Ok(())
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
