//! Integration tests for the codec-aware test harness drivers in
//! `wireframe_testing`.
#![cfg(not(loom))]

use std::{io, sync::Arc};

use bytes::Bytes;
use futures::future::BoxFuture;
use wireframe::{
    app::{Envelope, WireframeApp},
    codec::examples::{HotlineFrame, HotlineFrameCodec},
    serializer::{BincodeSerializer, Serializer},
};
use wireframe_testing::{
    decode_frames_with_codec,
    drive_with_codec_frames,
    drive_with_codec_payloads,
    drive_with_codec_payloads_mut,
    encode_payloads_with_codec,
    extract_payloads,
};

fn hotline_codec() -> HotlineFrameCodec { HotlineFrameCodec::new(4096) }

fn build_echo_app(
    codec: HotlineFrameCodec,
) -> io::Result<WireframeApp<BincodeSerializer, (), Envelope, HotlineFrameCodec>> {
    WireframeApp::<BincodeSerializer, (), Envelope>::new()
        .map_err(|e| io::Error::other(format!("app init: {e}")))?
        .with_codec(codec)
        .route(
            1,
            Arc::new(|_: &Envelope| -> BoxFuture<'static, ()> { Box::pin(async {}) }),
        )
        .map_err(|e| io::Error::other(format!("route: {e}")))
}

fn serialize_envelope(payload: &[u8]) -> io::Result<Vec<u8>> {
    let env = Envelope::new(1, Some(7), payload.to_vec());
    BincodeSerializer
        .serialize(&env)
        .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, format!("serialize: {e}")))
}

#[test]
fn encode_payloads_with_codec_produces_decodable_frames() -> io::Result<()> {
    let codec = hotline_codec();
    let payloads = vec![vec![1, 2, 3], vec![4, 5]];
    let encoded = encode_payloads_with_codec(&codec, payloads.clone())?;

    let wire: Vec<u8> = encoded.into_iter().flatten().collect();
    let frames = decode_frames_with_codec(&codec, wire)?;

    let extracted = extract_payloads::<HotlineFrameCodec>(&frames);
    if extracted != payloads {
        return Err(io::Error::other("round-trip payloads must match"));
    }
    Ok(())
}

#[test]
fn decode_frames_with_codec_handles_empty_input() -> io::Result<()> {
    let codec = hotline_codec();
    let frames = decode_frames_with_codec(&codec, vec![])?;
    if !frames.is_empty() {
        return Err(io::Error::other("empty input should produce no frames"));
    }
    Ok(())
}

#[test]
fn extract_payloads_returns_payload_bytes() {
    let frames = vec![
        HotlineFrame {
            transaction_id: 1,
            payload: Bytes::from_static(b"hello"),
        },
        HotlineFrame {
            transaction_id: 2,
            payload: Bytes::from_static(b"world"),
        },
    ];
    let payloads = extract_payloads::<HotlineFrameCodec>(&frames);
    assert_eq!(payloads, vec![b"hello".to_vec(), b"world".to_vec()]);
}

#[tokio::test]
async fn drive_with_codec_payloads_round_trips_through_echo_app() -> io::Result<()> {
    let codec = hotline_codec();
    let app = build_echo_app(codec.clone())?;

    let payload = vec![10, 20, 30];
    let serialized = serialize_envelope(&payload)?;

    let response_payloads = drive_with_codec_payloads(app, &codec, vec![serialized]).await?;

    if response_payloads.len() != 1 {
        return Err(io::Error::other(format!(
            "expected one response payload, got {}",
            response_payloads.len()
        )));
    }
    let first = response_payloads
        .first()
        .ok_or_else(|| io::Error::other("missing response payload"))?;
    let (decoded, _) = BincodeSerializer
        .deserialize::<Envelope>(first)
        .map_err(|e| io::Error::other(format!("deserialize: {e}")))?;
    if decoded.payload_bytes() != payload.as_slice() {
        return Err(io::Error::other("payload mismatch"));
    }
    Ok(())
}

#[tokio::test]
async fn drive_with_codec_frames_preserves_codec_metadata() -> io::Result<()> {
    let codec = hotline_codec();
    let app = build_echo_app(codec.clone())?;

    let serialized = serialize_envelope(&[42])?;

    let frames = drive_with_codec_frames(app, &codec, vec![serialized]).await?;

    if frames.len() != 1 {
        return Err(io::Error::other(format!(
            "expected one response frame, got {}",
            frames.len()
        )));
    }
    let frame = frames
        .first()
        .ok_or_else(|| io::Error::other("missing response frame"))?;
    // wrap_payload assigns transaction_id 0, confirming codec metadata flows
    // through the driver pipeline.
    if frame.transaction_id != 0 {
        return Err(io::Error::other(format!(
            "wrap_payload should assign transaction_id 0, got {}",
            frame.transaction_id
        )));
    }
    Ok(())
}

#[tokio::test]
async fn drive_with_codec_payloads_mut_allows_app_reuse() -> io::Result<()> {
    let codec = hotline_codec();
    let mut app = build_echo_app(codec.clone())?;

    let serialized = serialize_envelope(&[1])?;

    let first = drive_with_codec_payloads_mut(&mut app, &codec, vec![serialized.clone()]).await?;
    if first.is_empty() {
        return Err(io::Error::other("first call should produce output"));
    }

    let second = drive_with_codec_payloads_mut(&mut app, &codec, vec![serialized]).await?;
    if second.is_empty() {
        return Err(io::Error::other("second call should produce output"));
    }
    Ok(())
}
