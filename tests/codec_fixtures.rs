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

/// Decode `wire` with a fresh `HotlineFrameCodec` and verify that decoding
/// fails with an error message containing `expected_error_substring`.
fn assert_decode_fails_with(wire: Vec<u8>, expected_error_substring: &str) -> io::Result<()> {
    let codec = hotline_codec();
    let result = decode_frames_with_codec(&codec, wire);

    let err = result
        .err()
        .ok_or_else(|| io::Error::other("expected decode to fail but it succeeded"))?;
    if !err.to_string().contains(expected_error_substring) {
        return Err(io::Error::other(format!(
            "expected error containing '{expected_error_substring}', got: {err}"
        )));
    }
    Ok(())
}

/// Assert that `frames` contains exactly `expected` elements.
fn assert_frame_count(
    frames: &[wireframe::codec::examples::HotlineFrame],
    expected: usize,
) -> io::Result<()> {
    if frames.len() != expected {
        return Err(io::Error::other(format!(
            "expected {expected} frame(s), got {}",
            frames.len()
        )));
    }
    Ok(())
}

// ── Valid frame fixtures ────────────────────────────────────────────────

#[test]
fn valid_hotline_wire_decodes_successfully() -> io::Result<()> {
    let wire = valid_hotline_wire(b"hello", 7);
    let codec = hotline_codec();
    let frames = decode_frames_with_codec(&codec, wire)?;

    assert_frame_count(&frames, 1)?;

    let frame = frames
        .first()
        .ok_or_else(|| io::Error::other("expected one decoded frame"))?;

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
    assert_decode_fails_with(wire, "payload too large")
}

#[test]
fn mismatched_total_size_rejected_by_decoder() -> io::Result<()> {
    let wire = mismatched_total_size_wire(b"test");
    assert_decode_fails_with(wire, "invalid total size")
}

// ── Incomplete frame fixtures ───────────────────────────────────────────

#[test]
fn truncated_header_produces_decode_error() -> io::Result<()> {
    let wire = truncated_hotline_header();
    assert_decode_fails_with(wire, "bytes remaining")
}

#[test]
fn truncated_payload_produces_decode_error() -> io::Result<()> {
    let wire = truncated_hotline_payload(100);
    assert_decode_fails_with(wire, "bytes remaining")
}

// ── Correlation metadata fixtures ───────────────────────────────────────

#[test]
fn correlated_frames_share_transaction_id() -> io::Result<()> {
    let wire = correlated_hotline_wire(42, &[b"a", b"b", b"c"]);
    let codec = hotline_codec();
    let frames = decode_frames_with_codec(&codec, wire)?;

    assert_frame_count(&frames, 3)?;

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

    assert_frame_count(&frames, 3)?;

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
