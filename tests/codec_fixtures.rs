//! Integration tests for the codec fixture functions in `wireframe_testing`.
//!
//! These tests verify that each fixture category produces wire bytes with the
//! expected decoding behaviour when used with `HotlineFrameCodec`.
#![cfg(not(loom))]
#![forbid(unsafe_code)]

use std::io;

use rstest::rstest;
use wireframe::codec::{CodecError, EofError, examples::HotlineFrameCodec};
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

/// Decode `wire` and return the error the decoder must report.
fn decode_error(wire: &[u8]) -> io::Result<io::Error> {
    decode_frames_with_codec(&hotline_codec(), wire)
        .err()
        .ok_or_else(|| io::Error::other("expected decode to fail but it succeeded"))
}

/// Extract the structured end-of-file condition a decode error carries.
fn eof_condition(error: &io::Error) -> Option<EofError> {
    error
        .get_ref()?
        .downcast_ref::<CodecError>()
        .and_then(|codec_error| match codec_error {
            CodecError::Eof(eof) => Some(*eof),
            _ => None,
        })
}

/// Verify that decoding `wire` fails with an error satisfying `matches`.
fn assert_decode_fails(
    wire: &[u8],
    expected: &str,
    matches: impl Fn(&io::Error) -> bool,
) -> io::Result<()> {
    let err = decode_error(wire)?;
    if matches(&err) {
        return Ok(());
    }
    Err(io::Error::other(format!("expected {expected}, got: {err}")))
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
    let frames = decode_frames_with_codec(&codec, &wire)?;

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
    assert_eq!(
        frame.transaction_id, 42,
        "frame.transaction_id should equal 42"
    );
    assert_eq!(
        frame.payload.as_ref(),
        b"data",
        "frame.payload.as_ref() should equal b\"data\""
    );
}

// ── Invalid frame fixtures ──────────────────────────────────────────────

#[rstest]
#[case::oversized(oversized_hotline_wire(4096), "payload too large")]
#[case::mismatched(mismatched_total_size_wire(b"test"), "invalid total size")]
fn invalid_fixture_is_rejected(#[case] wire: Vec<u8>, #[case] expected: &str) -> io::Result<()> {
    assert_decode_fails(&wire, expected, |err| err.to_string().contains(expected))
}

// ── Incomplete frame fixtures ───────────────────────────────────────────

#[rstest]
#[case::header(
    truncated_hotline_header(),
    EofError::MidHeader {
        bytes_received: 10,
        header_size: 20,
    }
)]
#[case::payload(
    truncated_hotline_payload(100),
    EofError::MidFrame {
        bytes_received: 50,
        expected: 100,
    }
)]
fn truncated_fixture_reports_a_structured_eof(
    #[case] wire: Vec<u8>,
    #[case] expected: EofError,
) -> io::Result<()> {
    assert_decode_fails(&wire, &format!("{expected:?}"), |err| {
        eof_condition(err) == Some(expected)
    })
}

#[test]
fn truncated_payload_contains_only_header_and_partial_payload() {
    let wire = truncated_hotline_payload(8);
    let expected: &[u8] = &[
        // data_size = 8
        0, 0, 0, 8, // total_size = header (20) + data (8)
        0, 0, 0, 28, // transaction_id = 0
        0, 0, 0, 0, // reserved header bytes
        0, 0, 0, 0, 0, 0, 0, 0, // half of the declared payload
        0xcc, 0xcc, 0xcc, 0xcc,
    ];

    assert_eq!(
        wire.len(),
        expected.len(),
        "wire.len() should equal expected.len()"
    );
    assert_eq!(wire, expected, "wire should equal expected");
}

/// Verify each frame carries the expected transaction ID.
fn assert_transaction_ids(
    frames: &[wireframe::codec::examples::HotlineFrame],
    expected_ids: &[u32],
) -> io::Result<()> {
    assert_frame_count(frames, expected_ids.len())?;
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

// ── Correlation metadata fixtures ───────────────────────────────────────

#[test]
fn correlated_frames_share_transaction_id() -> io::Result<()> {
    let wire = correlated_hotline_wire(42, &[b"a", b"b", b"c"]);
    let codec = hotline_codec();
    let frames = decode_frames_with_codec(&codec, &wire)?;

    assert_frame_count(&frames, 3)?;
    assert_transaction_ids(&frames, &[42, 42, 42])
}

#[test]
fn sequential_frames_have_incrementing_ids() -> io::Result<()> {
    let wire = sequential_hotline_wire(10, &[b"x", b"y", b"z"]);
    let codec = hotline_codec();
    let frames = decode_frames_with_codec(&codec, &wire)?;

    assert_frame_count(&frames, 3)?;
    assert_transaction_ids(&frames, &[10, 11, 12])
}
