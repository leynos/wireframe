//! Tests for routing behaviour in `WireframeApp`.
//!
//! They validate handler invocation, echo responses, and sequential processing.
#![cfg(not(loom))]

use std::sync::{
    Arc,
    atomic::{AtomicUsize, Ordering},
};

use bytes::BytesMut;
use rstest::rstest;
use tokio_util::codec::Encoder;
use wireframe::{
    app::Packet,
    message::Message,
    serializer::{BincodeSerializer, Serializer},
};
use wireframe_testing::{
    CommonTestEnvelope,
    TEST_MAX_FRAME,
    TestResult,
    decode_frames,
    drive_with_bincode,
    drive_with_frames,
    new_test_codec,
};

type TestApp = wireframe::app::WireframeApp<BincodeSerializer, (), CommonTestEnvelope>;

#[derive(bincode::Encode, bincode::BorrowDecode, PartialEq, Debug)]
struct Echo(u8);

#[tokio::test]
async fn handler_receives_message_and_echoes_response() -> TestResult<()> {
    let called = Arc::new(AtomicUsize::new(0));
    let called_clone = called.clone();
    let app = TestApp::new()?.route(
        1,
        std::sync::Arc::new(move |_: &CommonTestEnvelope| {
            let called_inner = called_clone.clone();
            Box::pin(async move {
                called_inner.fetch_add(1, Ordering::SeqCst);
                // `WireframeApp` sends the envelope back automatically
            })
        }),
    )?;
    let msg_bytes = Echo(42).to_bytes()?;
    let env = CommonTestEnvelope {
        id: 1,
        correlation_id: Some(99),
        payload: msg_bytes,
    };

    let out = drive_with_bincode(app, env).await?;

    let frames = decode_frames(&out)?;
    let [first] = frames.as_slice() else {
        return Err("expected a single response frame".into());
    };
    let (resp_env, _) = BincodeSerializer.deserialize::<CommonTestEnvelope>(first)?;
    if resp_env.correlation_id != Some(99) {
        return Err(format!(
            "correlation id mismatch: expected Some(99), got {:?}",
            resp_env.correlation_id
        )
        .into());
    }
    let (echo, _) = Echo::from_bytes(&resp_env.payload)?;
    if echo != Echo(42) {
        return Err(format!("echo payload mismatch: expected Echo(42), got {echo:?}").into());
    }
    let call_count = called.load(Ordering::SeqCst);
    if call_count != 1 {
        return Err(format!("route not invoked exactly once: got {call_count} calls").into());
    }
    Ok(())
}

#[tokio::test]
async fn handler_echoes_with_none_correlation_id() -> TestResult<()> {
    let app = TestApp::new()?.route(
        1,
        std::sync::Arc::new(|_: &CommonTestEnvelope| Box::pin(async {})),
    )?;

    let msg_bytes = Echo(7).to_bytes()?;
    let env = CommonTestEnvelope {
        id: 1,
        correlation_id: None,
        payload: msg_bytes,
    };

    let out = drive_with_bincode(app, env).await?;
    let frames = decode_frames(&out)?;
    let [first] = frames.as_slice() else {
        return Err("expected a single response frame".into());
    };
    let (resp_env, _) = BincodeSerializer.deserialize::<CommonTestEnvelope>(first)?;

    if resp_env.correlation_id.is_some() {
        return Err(format!(
            "unexpected correlation id: got {:?}",
            resp_env.correlation_id
        )
        .into());
    }
    let (echo, _) = Echo::from_bytes(&resp_env.payload)?;
    if echo != Echo(7) {
        return Err(format!("echo payload mismatch: expected Echo(7), got {echo:?}").into());
    }
    Ok(())
}

#[tokio::test]
async fn multiple_frames_processed_in_sequence() -> TestResult<()> {
    let app = TestApp::new()?.route(
        1,
        std::sync::Arc::new(|_: &CommonTestEnvelope| Box::pin(async {})),
    )?;

    let mut codec = new_test_codec(TEST_MAX_FRAME);
    let mut encoded_frames = Vec::new();
    for id in 1u8..=2 {
        let msg_bytes = Echo(id).to_bytes()?;
        let env = CommonTestEnvelope {
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

    let frames = decode_frames(&out)?;
    let [first, second] = frames.as_slice() else {
        return Err("expected two response frames".into());
    };
    let (env1, _) = BincodeSerializer.deserialize::<CommonTestEnvelope>(first)?;
    let (echo1, _) = Echo::from_bytes(&env1.payload)?;
    let (env2, _) = BincodeSerializer.deserialize::<CommonTestEnvelope>(second)?;
    let (echo2, _) = Echo::from_bytes(&env2.payload)?;
    if env1.correlation_id != Some(1) {
        return Err(format!(
            "first correlation id mismatch: expected Some(1), got {:?}",
            env1.correlation_id
        )
        .into());
    }
    if env2.correlation_id != Some(2) {
        return Err(format!(
            "second correlation id mismatch: expected Some(2), got {:?}",
            env2.correlation_id
        )
        .into());
    }
    if echo1 != Echo(1) {
        return Err(format!("first echo payload mismatch: expected Echo(1), got {echo1:?}").into());
    }
    if echo2 != Echo(2) {
        return Err(
            format!("second echo payload mismatch: expected Echo(2), got {echo2:?}").into(),
        );
    }
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
        std::sync::Arc::new(|_: &CommonTestEnvelope| Box::pin(async {})),
    )?;

    let msg_bytes = Echo(5).to_bytes()?;
    let env = CommonTestEnvelope {
        id: 1,
        correlation_id: cid,
        payload: msg_bytes,
    };
    let env_bytes = BincodeSerializer.serialize(&env)?;

    let mut frame_buf = BytesMut::with_capacity(env_bytes.len() + 4);
    let mut codec = new_test_codec(TEST_MAX_FRAME);
    codec.encode(env_bytes.into(), &mut frame_buf)?;

    let out = drive_with_frames(app, vec![frame_buf.to_vec()]).await?;
    let frames = decode_frames(&out)?;
    let [first] = frames.as_slice() else {
        return Err("expected a single response frame".into());
    };
    let (resp, _) = BincodeSerializer.deserialize::<CommonTestEnvelope>(first)?;

    if resp.correlation_id != cid {
        return Err(format!(
            "correlation id mismatch: expected {cid:?}, got {:?}",
            resp.correlation_id
        )
        .into());
    }
    Ok(())
}

#[test]
fn packet_from_parts_round_trips() {
    let env = CommonTestEnvelope {
        id: 5,
        correlation_id: Some(9),
        payload: vec![1, 2, 3],
    };
    let parts = env.clone().into_parts();
    let rebuilt = CommonTestEnvelope::from_parts(parts);
    assert_eq!(rebuilt, env);
}
