//! Direct tests for the recovery-aware decoder adapter.
//!
//! Each case drives one `Decoder` call, so the assertions pin the three facts
//! the connection loop relies on: whether the call produced a frame or a
//! captured failure, which entry point produced it, and how many bytes the
//! failing call removed from the read buffer. The consumption count is what
//! lets the loop refuse a drop that would repeat the same bytes forever.

use std::io;

use bytes::{Buf, BytesMut};
use tokio_util::codec::Decoder;

use super::{DecodeOrigin, DecodedItem, RecoveryDecoder};

/// Decoder with a one-byte length prefix and two deliberate failures.
///
/// A prefix of [`REJECTED`] makes `decode` consume the frame before failing, a
/// prefix of [`UNCONSUMABLE`] makes it fail without consuming anything, and any
/// other value is the payload length of a complete frame. `decode_eof` treats
/// every leftover byte as a truncated frame.
struct ScriptedDecoder;

/// Prefix marking a frame the decoder reads and then rejects.
const REJECTED: u8 = 0xff;

/// Prefix marking bytes the decoder refuses without consuming.
const UNCONSUMABLE: u8 = 0xfe;

impl Decoder for ScriptedDecoder {
    type Item = Vec<u8>;
    type Error = io::Error;

    fn decode(&mut self, src: &mut BytesMut) -> Result<Option<Self::Item>, Self::Error> {
        let Some(&prefix) = src.first() else {
            return Ok(None);
        };
        match prefix {
            REJECTED => {
                // A decoder that has already advanced past the malformed frame
                // is what makes a drop safe; the failure has to report that.
                let taken = src.len().min(2);
                src.advance(taken);
                Err(invalid_data("rejected frame"))
            }
            UNCONSUMABLE => Err(invalid_data("unframed bytes")),
            length => {
                let needed = 1 + usize::from(length);
                if src.len() < needed {
                    return Ok(None);
                }
                let mut frame = src.split_to(needed);
                frame.advance(1);
                Ok(Some(frame.to_vec()))
            }
        }
    }

    fn decode_eof(&mut self, src: &mut BytesMut) -> Result<Option<Self::Item>, Self::Error> {
        if src.is_empty() {
            return Ok(None);
        }
        // The peer stopped mid-frame. Consuming the partial frame leaves the
        // decoder at a boundary, and the failure reports the removed bytes.
        let taken = src.len();
        src.advance(taken);
        Err(truncated())
    }
}

/// Build the error a rejected frame reports.
fn invalid_data(message: &str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, message.to_owned())
}

/// Build the error a truncated frame at end of input reports.
fn truncated() -> io::Error { io::Error::new(io::ErrorKind::UnexpectedEof, "truncated frame") }

/// Split a captured failure into its origin, consumption, and original error.
///
/// `into_parts` reports the error first, the way the failure stores it; the
/// call sites read better with the two measurements leading.
fn failure_parts(item: DecodedItem<Vec<u8>>) -> (DecodeOrigin, usize, io::Error) {
    match item {
        DecodedItem::Frame(frame) => panic!("expected a captured failure, decoded {frame:?}"),
        DecodedItem::Failure(failure) => {
            let (error, origin, consumed) = failure.into_parts();
            (origin, consumed, error)
        }
    }
}

/// Assert a decode result carries the expected payload.
fn expect_frame(item: Option<DecodedItem<Vec<u8>>>, expected: &[u8]) {
    match item {
        Some(DecodedItem::Frame(payload)) if payload == expected => {}
        Some(DecodedItem::Frame(payload)) => {
            panic!("decoded {payload:?}, expected {expected:?}");
        }
        Some(DecodedItem::Failure(failure)) => {
            panic!("expected a frame, captured {failure:?}");
        }
        None => panic!("expected a frame, decoded nothing"),
    }
}

/// A complete frame is passed through and its bytes are consumed.
#[test]
fn a_complete_frame_is_passed_through() {
    let mut decoder = RecoveryDecoder::new(ScriptedDecoder);
    let mut src = BytesMut::from(&[2_u8, b'a', b'b'][..]);

    expect_frame(
        decoder.decode(&mut src).expect("a complete frame decodes"),
        b"ab",
    );
    assert!(src.is_empty(), "the decoded frame's bytes are consumed");
}

/// An incomplete frame yields no item and leaves its bytes buffered.
#[test]
fn an_incomplete_frame_is_buffered_until_more_arrives() {
    let mut decoder = RecoveryDecoder::new(ScriptedDecoder);
    let mut src = BytesMut::from(&[2_u8, b'a'][..]);

    let first = decoder
        .decode(&mut src)
        .expect("an incomplete frame is not an error");
    assert!(first.is_none(), "an incomplete frame must yield no item");
    assert_eq!(
        src.len(),
        2,
        "incomplete bytes stay buffered for the next read"
    );

    src.extend_from_slice(b"b");
    expect_frame(
        decoder
            .decode(&mut src)
            .expect("the completed frame decodes"),
        b"ab",
    );
    assert!(src.is_empty(), "the completed frame's bytes are consumed");
}

/// A failing `decode` reports the decode origin and the bytes it consumed.
#[test]
fn a_decode_failure_reports_the_decode_origin_and_consumed_bytes() {
    let mut decoder = RecoveryDecoder::new(ScriptedDecoder);
    let mut src = BytesMut::from(&[REJECTED, b'x', b'y'][..]);

    let item = decoder
        .decode(&mut src)
        .expect("a failure is captured, not raised");
    let (origin, consumed, error) =
        failure_parts(item.expect("the failure is reported as an item"));

    assert_eq!(origin, DecodeOrigin::Decode);
    assert_eq!(
        consumed, 2,
        "the failing call removed its frame from the buffer"
    );
    assert_eq!(
        error.kind(),
        io::ErrorKind::InvalidData,
        "the original kind survives"
    );
    assert_eq!(
        error.to_string(),
        "rejected frame",
        "the original message survives"
    );
    assert_eq!(
        src.as_ref(),
        b"y",
        "only the failing frame's bytes are removed"
    );
}

/// A failing `decode` that consumed nothing reports exactly zero.
#[test]
fn a_failure_that_consumed_nothing_reports_zero() {
    let mut decoder = RecoveryDecoder::new(ScriptedDecoder);
    let mut src = BytesMut::from(&[UNCONSUMABLE, b'a', b'b'][..]);

    let item = decoder
        .decode(&mut src)
        .expect("a failure is captured, not raised");
    let (origin, consumed, _) = failure_parts(item.expect("the failure is reported as an item"));

    assert_eq!(origin, DecodeOrigin::Decode);
    assert_eq!(
        consumed, 0,
        "a decoder that left the bytes in place consumed none"
    );
    assert_eq!(src.len(), 3, "the refused bytes are still buffered");
}

/// A failing `decode_eof` reports the EOF origin and the bytes it consumed.
#[test]
fn an_eof_failure_reports_the_eof_origin_and_consumed_bytes() {
    let mut decoder = RecoveryDecoder::new(ScriptedDecoder);
    let mut src = BytesMut::from(&[2_u8, b'a'][..]);

    let item = decoder
        .decode_eof(&mut src)
        .expect("a failure is captured, not raised");
    let (origin, consumed, error) =
        failure_parts(item.expect("the failure is reported as an item"));

    assert_eq!(
        origin,
        DecodeOrigin::DecodeEof,
        "the EOF entry point is named"
    );
    assert_eq!(consumed, 2, "the truncated frame is measured");
    assert_eq!(error.kind(), io::ErrorKind::UnexpectedEof);
    assert!(
        src.is_empty(),
        "the truncated frame is removed from the buffer"
    );
}

/// End of input at a frame boundary is a clean close, not a failure.
#[test]
fn an_empty_buffer_at_eof_yields_no_item() {
    let mut decoder = RecoveryDecoder::new(ScriptedDecoder);
    let mut src = BytesMut::new();

    let item = decoder
        .decode_eof(&mut src)
        .expect("a clean close is not an error");
    assert!(item.is_none(), "a clean close yields no item");
}

/// A captured failure does not stop the decoder from reading the next frame.
#[test]
fn a_failure_does_not_stop_the_decoder_from_resuming() {
    let mut decoder = RecoveryDecoder::new(ScriptedDecoder);
    let mut src = BytesMut::from(&[REJECTED, b'x', 1_u8, b'z'][..]);

    let item = decoder
        .decode(&mut src)
        .expect("a failure is captured, not raised");
    let (_, consumed, _) = failure_parts(item.expect("the failure is reported as an item"));
    assert_eq!(consumed, 2);

    expect_frame(
        decoder.decode(&mut src).expect("the next frame decodes"),
        b"z",
    );
}
