//! Integration tests for partial-frame and fragment feeding utilities in
//! `wireframe_testing`.
#![cfg(not(loom))]

use std::{io, num::NonZeroUsize, sync::Arc};

use futures::future::BoxFuture;
use wireframe::{
    app::{Envelope, WireframeApp},
    codec::examples::HotlineFrameCodec,
    fragment::Fragmenter,
    serializer::{BincodeSerializer, Serializer},
};
use wireframe_testing::{
    drive_with_fragment_frames,
    drive_with_fragments,
    drive_with_fragments_mut,
    drive_with_partial_codec_frames,
    drive_with_partial_fragments,
    drive_with_partial_frames,
    drive_with_partial_frames_mut,
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

// ---------------------------------------------------------------------------
// Chunked-write (partial frame) tests
// ---------------------------------------------------------------------------

async fn test_partial_frames_with_chunk(payload: &[u8], chunk_size: usize) -> io::Result<()> {
    let codec = hotline_codec();
    let app = build_echo_app(codec.clone())?;
    let serialized = serialize_envelope(payload)?;
    let chunk = NonZeroUsize::new(chunk_size).ok_or_else(|| io::Error::other("non-zero"))?;

    let payloads = drive_with_partial_frames(app, &codec, vec![serialized], chunk).await?;

    if payloads.is_empty() {
        return Err(io::Error::other("expected non-empty response payloads"));
    }
    Ok(())
}

#[tokio::test]
async fn partial_frames_single_byte_chunks() -> io::Result<()> {
    test_partial_frames_with_chunk(&[10, 20, 30], 1).await
}

#[tokio::test]
async fn partial_frames_misaligned_chunks() -> io::Result<()> {
    test_partial_frames_with_chunk(&[1, 2, 3, 4, 5], 7).await
}

#[tokio::test]
async fn partial_frames_multiple_payloads() -> io::Result<()> {
    let codec = hotline_codec();
    let app = build_echo_app(codec.clone())?;
    let p1 = serialize_envelope(&[1])?;
    let p2 = serialize_envelope(&[2])?;
    let chunk = NonZeroUsize::new(3).ok_or_else(|| io::Error::other("non-zero"))?;

    let payloads = drive_with_partial_frames(app, &codec, vec![p1, p2], chunk).await?;

    if payloads.len() != 2 {
        return Err(io::Error::other(format!(
            "expected 2 response payloads, got {}",
            payloads.len()
        )));
    }
    Ok(())
}

#[tokio::test]
async fn partial_codec_frames_preserves_metadata() -> io::Result<()> {
    let codec = hotline_codec();
    let app = build_echo_app(codec.clone())?;
    let serialized = serialize_envelope(&[42])?;
    let chunk = NonZeroUsize::new(5).ok_or_else(|| io::Error::other("non-zero"))?;

    let frames = drive_with_partial_codec_frames(app, &codec, vec![serialized], chunk).await?;

    let frame = frames
        .first()
        .ok_or_else(|| io::Error::other("expected at least one response frame"))?;
    if frame.transaction_id != 0 {
        return Err(io::Error::other(format!(
            "wrap_payload should assign transaction_id 0, got {}",
            frame.transaction_id
        )));
    }
    Ok(())
}

#[tokio::test]
async fn partial_frames_mut_allows_reuse() -> io::Result<()> {
    let codec = hotline_codec();
    let mut app = build_echo_app(codec.clone())?;
    let serialized = serialize_envelope(&[1])?;
    let chunk = NonZeroUsize::new(2).ok_or_else(|| io::Error::other("non-zero"))?;

    let first =
        drive_with_partial_frames_mut(&mut app, &codec, vec![serialized.clone()], chunk).await?;
    if first.is_empty() {
        return Err(io::Error::other("first call should produce output"));
    }

    let second = drive_with_partial_frames_mut(&mut app, &codec, vec![serialized], chunk).await?;
    if second.is_empty() {
        return Err(io::Error::other("second call should produce output"));
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// Fragment feeding tests
// ---------------------------------------------------------------------------

// Fragment payloads are FRAG-prefixed raw bytes, not valid Envelope
// serializations. The app receives and processes them but does not produce a
// routed response. Verifying no I/O error confirms the full transport pipeline
// (fragment → encode → transport → decode) works end to end.

#[tokio::test]
async fn fragment_round_trip() -> io::Result<()> {
    let codec = hotline_codec();
    let app = build_echo_app(codec.clone())?;
    let cap = NonZeroUsize::new(20).ok_or_else(|| io::Error::other("non-zero"))?;
    let fragmenter = Fragmenter::new(cap);

    let _payloads = drive_with_fragments(app, &codec, &fragmenter, vec![0; 100]).await?;
    Ok(())
}

#[tokio::test]
async fn fragment_frames_returns_codec_frames() -> io::Result<()> {
    let codec = hotline_codec();
    let app = build_echo_app(codec.clone())?;
    let cap = NonZeroUsize::new(20).ok_or_else(|| io::Error::other("non-zero"))?;
    let fragmenter = Fragmenter::new(cap);

    let _frames = drive_with_fragment_frames(app, &codec, &fragmenter, vec![0; 50]).await?;
    Ok(())
}

#[tokio::test]
async fn fragment_mut_allows_reuse() -> io::Result<()> {
    let codec = hotline_codec();
    let mut app = build_echo_app(codec.clone())?;
    let cap = NonZeroUsize::new(30).ok_or_else(|| io::Error::other("non-zero"))?;
    let fragmenter = Fragmenter::new(cap);

    let _first = drive_with_fragments_mut(&mut app, &codec, &fragmenter, vec![0; 50]).await?;
    let _second = drive_with_fragments_mut(&mut app, &codec, &fragmenter, vec![0; 30]).await?;
    Ok(())
}

#[tokio::test]
async fn partial_fragments_combines_both() -> io::Result<()> {
    let codec = hotline_codec();
    let app = build_echo_app(codec.clone())?;
    let cap = NonZeroUsize::new(20).ok_or_else(|| io::Error::other("non-zero"))?;
    let fragmenter = Fragmenter::new(cap);
    let chunk = NonZeroUsize::new(3).ok_or_else(|| io::Error::other("non-zero"))?;

    let _payloads =
        drive_with_partial_fragments(app, &codec, &fragmenter, vec![0; 100], chunk).await?;
    Ok(())
}
