//! Integration tests for the codec fixture functions in `wireframe_testing`.
//!
//! These tests verify that each fixture category produces wire bytes with the
//! expected decoding behaviour when used with `HotlineFrameCodec`.
#![cfg(not(loom))]

use std::io;

use wireframe::codec::examples::HotlineFrameCodec;
use wireframe_testing::{
    correlated_hotline_wire,
    decode_frames_with_codec,
    mismatched_total_size_wire,
    oversized_hotline_wire,
    sequential_hotline_wire,
    truncated_hotline_header,
    truncated_hotline_payload,
    valid_hotline_frame,
    valid_hotline_wire,
};

fn hotline_codec() -> HotlineFrameCodec { HotlineFrameCodec::new(4096) }

// ── Valid frame fixtures ────────────────────────────────────────────────

#[test]
fn valid_hotline_wire_decodes_successfully() -> io::Result<()> {
    let wire = valid_hotline_wire(b"hello", 7);
    let codec = hotline_codec();
    let frames = decode_frames_with_codec(&codec, wire)?;

    let frame = frames
        .first()
        .ok_or_else(|| io::Error::other("expected one decoded frame"))?;

    if frames.len() != 1 {
        return Err(io::Error::other(format!(
            "expected 1 frame, got {}",
            frames.len()
        )));
    }
    if frame.transaction_id != 7 {
        return Err(io::Error::other(format!(
            "expected transaction_id 7, got {}",
            frame.transaction_id
        )));
    }
    if frame.payload.as_ref() != b"hello" {
        return Err(io::Error::other("payload mismatch"));
    }
    Ok(())
}

#[test]
fn valid_hotline_frame_has_correct_metadata() {
    let frame = valid_hotline_frame(b"data", 42);
    assert_eq!(frame.transaction_id, 42);
    assert_eq!(frame.payload.as_ref(), b"data");
}

// ── Invalid frame fixtures ──────────────────────────────────────────────

#[test]
fn oversized_hotline_wire_rejected_by_decoder() -> io::Result<()> {
    let wire = oversized_hotline_wire(4096);
    let codec = hotline_codec();
    let result = decode_frames_with_codec(&codec, wire);

    let err = result
        .err()
        .ok_or_else(|| io::Error::other("expected decode to fail for oversized frame"))?;
    if !err.to_string().contains("payload too large") {
        return Err(io::Error::other(format!(
            "expected 'payload too large' error, got: {err}"
        )));
    }
    Ok(())
}

#[test]
fn mismatched_total_size_rejected_by_decoder() -> io::Result<()> {
    let wire = mismatched_total_size_wire(b"test");
    let codec = hotline_codec();
    let result = decode_frames_with_codec(&codec, wire);

    let err = result
        .err()
        .ok_or_else(|| io::Error::other("expected decode to fail for mismatched total_size"))?;
    if !err.to_string().contains("invalid total size") {
        return Err(io::Error::other(format!(
            "expected 'invalid total size' error, got: {err}"
        )));
    }
    Ok(())
}

// ── Incomplete frame fixtures ───────────────────────────────────────────

#[test]
fn truncated_header_produces_decode_error() -> io::Result<()> {
    let wire = truncated_hotline_header();
    let codec = hotline_codec();
    let result = decode_frames_with_codec(&codec, wire);

    let err = result
        .err()
        .ok_or_else(|| io::Error::other("expected decode to fail for truncated header"))?;
    // The default decode_eof sees non-empty buf and returns
    // "bytes remaining on stream", which decode_frames_with_codec wraps.
    if !err.to_string().contains("bytes remaining") {
        return Err(io::Error::other(format!(
            "expected 'bytes remaining' error, got: {err}"
        )));
    }
    Ok(())
}

#[test]
fn truncated_payload_produces_decode_error() -> io::Result<()> {
    let wire = truncated_hotline_payload(100);
    let codec = hotline_codec();
    let result = decode_frames_with_codec(&codec, wire);

    let err = result
        .err()
        .ok_or_else(|| io::Error::other("expected decode to fail for truncated payload"))?;
    // The default decode_eof sees non-empty buf and returns
    // "bytes remaining on stream", which decode_frames_with_codec wraps.
    if !err.to_string().contains("bytes remaining") {
        return Err(io::Error::other(format!(
            "expected 'bytes remaining' error, got: {err}"
        )));
    }
    Ok(())
}

// ── Correlation metadata fixtures ───────────────────────────────────────

#[test]
fn correlated_frames_share_transaction_id() -> io::Result<()> {
    let wire = correlated_hotline_wire(42, &[b"a", b"b", b"c"]);
    let codec = hotline_codec();
    let frames = decode_frames_with_codec(&codec, wire)?;

    if frames.len() != 3 {
        return Err(io::Error::other(format!(
            "expected 3 frames, got {}",
            frames.len()
        )));
    }
    for (i, frame) in frames.iter().enumerate() {
        if frame.transaction_id != 42 {
            return Err(io::Error::other(format!(
                "frame {i}: expected transaction_id 42, got {}",
                frame.transaction_id
            )));
        }
    }
    Ok(())
}

#[test]
fn sequential_frames_have_incrementing_ids() -> io::Result<()> {
    let wire = sequential_hotline_wire(10, &[b"x", b"y", b"z"]);
    let codec = hotline_codec();
    let frames = decode_frames_with_codec(&codec, wire)?;

    if frames.len() != 3 {
        return Err(io::Error::other(format!(
            "expected 3 frames, got {}",
            frames.len()
        )));
    }
    let expected_ids: &[u32] = &[10, 11, 12];
    for (i, (frame, expected_id)) in frames.iter().zip(expected_ids.iter()).enumerate() {
        if frame.transaction_id != *expected_id {
            return Err(io::Error::other(format!(
                "frame {i}: expected transaction_id {expected_id}, got {}",
                frame.transaction_id
            )));
        }
    }
    Ok(())
}
