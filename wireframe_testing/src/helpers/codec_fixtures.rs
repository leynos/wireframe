//! Codec fixture functions for generating valid and invalid Hotline-framed
//! wire bytes.
//!
//! These helpers produce raw byte sequences suitable for feeding to
//! [`decode_frames_with_codec`](super::decode_frames_with_codec) or directly
//! to a `HotlineAdapter` decoder. Four categories of fixtures are provided:
//!
//! - **Valid frames** — well-formed wire bytes that decode cleanly.
//! - **Invalid frames** — wire bytes triggering decoder errors (oversized payloads, mismatched
//!   sizes).
//! - **Incomplete frames** — truncated data that causes trailing-byte errors.
//! - **Correlation metadata** — multi-frame sequences with specific transaction IDs for correlation
//!   testing.
//!
//! Fixtures construct raw bytes directly rather than using the tokio-util
//! encoder, ensuring they are independent of the encoder implementation and
//! can represent malformed data that the encoder would reject.

use bytes::Bytes;
use wireframe::codec::{
    FrameCodec,
    examples::{HotlineFrame, HotlineFrameCodec},
};

/// Hotline header length in bytes: `data_size` (4) + `total_size` (4) +
/// `transaction_id` (4) + reserved (8).
const HEADER_LEN: usize = 20;

/// Build a single valid Hotline frame as raw wire bytes.
///
/// Writes the 20-byte Hotline header (`data_size`, `total_size`,
/// `transaction_id`, 8 reserved zero bytes) followed by `payload`.
///
/// # Examples
///
/// ```rust
/// use wireframe::codec::examples::HotlineFrameCodec;
/// use wireframe_testing::{decode_frames_with_codec, valid_hotline_wire};
///
/// let wire = valid_hotline_wire(b"hello", 7);
/// let codec = HotlineFrameCodec::new(4096);
/// let frames = decode_frames_with_codec(&codec, wire).expect("valid fixture should decode");
/// assert_eq!(frames.len(), 1);
/// ```
#[must_use]
pub fn valid_hotline_wire(payload: &[u8], transaction_id: u32) -> Vec<u8> {
    build_hotline_wire(payload, transaction_id, payload.len(), true)
}

/// Return a typed [`HotlineFrame`] with the given payload and transaction ID.
///
/// Useful when a test needs to inspect frame metadata without going through
/// the wire-encode/decode cycle.
///
/// # Examples
///
/// ```rust
/// use wireframe_testing::valid_hotline_frame;
///
/// let frame = valid_hotline_frame(b"data", 42);
/// assert_eq!(frame.transaction_id, 42);
/// assert_eq!(&*frame.payload, b"data");
/// ```
#[must_use]
pub fn valid_hotline_frame(payload: &[u8], transaction_id: u32) -> HotlineFrame {
    let codec = HotlineFrameCodec::new(payload.len());
    let mut frame = codec.wrap_payload(Bytes::copy_from_slice(payload));
    frame.transaction_id = transaction_id;
    frame
}

/// Build a Hotline frame whose `data_size` exceeds `max_frame_length` by one
/// byte.
///
/// The Hotline decoder rejects frames where `data_size > max_frame_length`
/// with an `InvalidData("payload too large")` error.
///
/// # Examples
///
/// ```rust
/// use wireframe::codec::examples::HotlineFrameCodec;
/// use wireframe_testing::{decode_frames_with_codec, oversized_hotline_wire};
///
/// let wire = oversized_hotline_wire(64);
/// let codec = HotlineFrameCodec::new(64);
/// let err =
///     decode_frames_with_codec(&codec, wire).expect_err("oversized frame should be rejected");
/// assert!(err.to_string().contains("payload too large"));
/// ```
#[must_use]
pub fn oversized_hotline_wire(max_frame_length: usize) -> Vec<u8> {
    let oversized_len = max_frame_length + 1;
    build_hotline_wire(&vec![0xab; oversized_len], 0, oversized_len, true)
}

/// Build a Hotline frame with a mismatched `total_size` field.
///
/// The header's `total_size` is set to `data_size + 21` (one byte more than
/// the correct value of `data_size + 20`). The Hotline decoder rejects this
/// with an `InvalidData("invalid total size")` error.
///
/// # Examples
///
/// ```rust
/// use wireframe::codec::examples::HotlineFrameCodec;
/// use wireframe_testing::{decode_frames_with_codec, mismatched_total_size_wire};
///
/// let wire = mismatched_total_size_wire(b"test");
/// let codec = HotlineFrameCodec::new(4096);
/// let err = decode_frames_with_codec(&codec, wire)
///     .expect_err("mismatched total_size should be rejected");
/// assert!(err.to_string().contains("invalid total size"));
/// ```
#[must_use]
pub fn mismatched_total_size_wire(payload: &[u8]) -> Vec<u8> {
    build_hotline_wire(payload, 0, payload.len(), false)
}

/// Return fewer than 20 bytes — a truncated Hotline header.
///
/// The Hotline decoder returns `Ok(None)` for this input (not enough bytes
/// to parse the header). When passed to
/// [`decode_frames_with_codec`](super::decode_frames_with_codec), the
/// `decode_eof` call detects unconsumed bytes and produces an
/// `InvalidData` error containing "bytes remaining".
///
/// # Examples
///
/// ```rust
/// use wireframe::codec::examples::HotlineFrameCodec;
/// use wireframe_testing::{decode_frames_with_codec, truncated_hotline_header};
///
/// let wire = truncated_hotline_header();
/// let codec = HotlineFrameCodec::new(4096);
/// let err = decode_frames_with_codec(&codec, wire)
///     .expect_err("truncated header should cause a decode error");
/// assert!(err.to_string().contains("bytes remaining"));
/// ```
#[must_use]
pub fn truncated_hotline_header() -> Vec<u8> {
    // 10 bytes — enough to look like the start of a header but too short
    // for the decoder to extract the full 20-byte header.
    vec![0; 10]
}

/// Return a valid Hotline header claiming `payload_len` bytes of payload,
/// but provide only half the payload bytes.
///
/// The Hotline decoder returns `Ok(None)` because the buffer is shorter than
/// `total_size`. When passed to
/// [`decode_frames_with_codec`](super::decode_frames_with_codec), the
/// `decode_eof` call detects unconsumed bytes and produces an
/// `InvalidData` error containing "bytes remaining".
///
/// # Examples
///
/// ```rust
/// use wireframe::codec::examples::HotlineFrameCodec;
/// use wireframe_testing::{decode_frames_with_codec, truncated_hotline_payload};
///
/// let wire = truncated_hotline_payload(100);
/// let codec = HotlineFrameCodec::new(4096);
/// let err = decode_frames_with_codec(&codec, wire)
///     .expect_err("truncated payload should cause a decode error");
/// assert!(err.to_string().contains("bytes remaining"));
/// ```
#[must_use]
pub fn truncated_hotline_payload(payload_len: usize) -> Vec<u8> {
    let data_size = u32_from_usize(payload_len);
    let total_size = u32_from_usize(payload_len + HEADER_LEN);

    let half_payload = payload_len / 2;
    let mut buf = Vec::with_capacity(HEADER_LEN + half_payload);
    buf.extend_from_slice(&data_size.to_be_bytes());
    buf.extend_from_slice(&total_size.to_be_bytes());
    buf.extend_from_slice(&0u32.to_be_bytes()); // transaction_id
    buf.extend_from_slice(&[0u8; 8]); // reserved
    buf.extend_from_slice(&vec![0xcc; half_payload]);
    buf
}

/// Encode multiple Hotline frames sharing the same `transaction_id`.
///
/// Suitable for verifying that correlation ID propagation works across frame
/// sequences. All frames in the returned byte vector carry the same
/// transaction identifier.
///
/// # Examples
///
/// ```rust
/// use wireframe::codec::examples::HotlineFrameCodec;
/// use wireframe_testing::{correlated_hotline_wire, decode_frames_with_codec};
///
/// let wire = correlated_hotline_wire(42, &[b"a", b"b"]);
/// let codec = HotlineFrameCodec::new(4096);
/// let frames = decode_frames_with_codec(&codec, wire).expect("correlated fixtures should decode");
/// assert_eq!(frames.len(), 2);
/// assert!(frames.iter().all(|f| f.transaction_id == 42));
/// ```
#[must_use]
pub fn correlated_hotline_wire(transaction_id: u32, payloads: &[&[u8]]) -> Vec<u8> {
    let mut buf = Vec::new();
    for payload in payloads {
        buf.extend_from_slice(&valid_hotline_wire(payload, transaction_id));
    }
    buf
}

/// Encode multiple Hotline frames with incrementing transaction IDs.
///
/// The first frame carries `base_transaction_id`, the second
/// `base_transaction_id + 1`, and so on. Suitable for verifying frame
/// ordering or sequential correlation.
///
/// # Examples
///
/// ```rust
/// use wireframe::codec::examples::HotlineFrameCodec;
/// use wireframe_testing::{decode_frames_with_codec, sequential_hotline_wire};
///
/// let wire = sequential_hotline_wire(10, &[b"x", b"y", b"z"]);
/// let codec = HotlineFrameCodec::new(4096);
/// let frames = decode_frames_with_codec(&codec, wire).expect("sequential fixtures should decode");
/// assert_eq!(frames.len(), 3);
/// ```
#[must_use]
pub fn sequential_hotline_wire(base_transaction_id: u32, payloads: &[&[u8]]) -> Vec<u8> {
    let mut buf = Vec::new();
    for (i, payload) in payloads.iter().enumerate() {
        #[expect(
            clippy::cast_possible_truncation,
            reason = "fixture payloads slice length will not exceed u32::MAX"
        )]
        let tid = base_transaction_id + i as u32;
        buf.extend_from_slice(&valid_hotline_wire(payload, tid));
    }
    buf
}

// ── Internal helpers ────────────────────────────────────────────────────

/// Construct a Hotline wire frame with explicit control over header fields.
///
/// When `correct_total_size` is `true`, `total_size` is set to
/// `data_size + HEADER_LEN`. When `false`, `total_size` is set to
/// `data_size + HEADER_LEN + 1` (deliberately wrong).
fn build_hotline_wire(
    payload: &[u8],
    transaction_id: u32,
    data_size: usize,
    correct_total_size: bool,
) -> Vec<u8> {
    let data_size_u32 = u32_from_usize(data_size);
    let total_size = if correct_total_size {
        u32_from_usize(data_size + HEADER_LEN)
    } else {
        // Off by one — triggers "invalid total size" in the decoder.
        u32_from_usize(data_size + HEADER_LEN + 1)
    };

    let mut buf = Vec::with_capacity(HEADER_LEN + payload.len());
    buf.extend_from_slice(&data_size_u32.to_be_bytes());
    buf.extend_from_slice(&total_size.to_be_bytes());
    buf.extend_from_slice(&transaction_id.to_be_bytes());
    buf.extend_from_slice(&[0u8; 8]); // reserved
    buf.extend_from_slice(payload);
    buf
}

/// Convert a `usize` to `u32`, saturating at `u32::MAX`.
///
/// Fixture payloads are always small enough to fit in `u32`, but we avoid a
/// truncating cast to satisfy Clippy.
fn u32_from_usize(value: usize) -> u32 { u32::try_from(value).unwrap_or(u32::MAX) }
