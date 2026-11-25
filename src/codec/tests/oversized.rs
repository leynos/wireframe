//! Oversized length-prefix classification for the length-delimited codec.
//!
//! Tokio-util rejects a frame whose declared length exceeds the codec limit as
//! soon as the prefix is complete. Both entry points must report that failure
//! as the same structured framing error: the recovery hook selects its policy
//! from the error type, so a plain `io` error would apply the wrong policy to
//! an oversized frame that arrives before a close.

use std::io;

use bytes::BytesMut;
use rstest::rstest;

use super::*;

/// Entry point a decoder reports the oversized prefix through.
#[derive(Clone, Copy, Debug)]
enum EntryPoint {
    /// [`Decoder::decode`], the ordinary read path.
    Decode,
    /// [`Decoder::decode_eof`], the path taken once the peer stops sending.
    DecodeEof,
}

/// Run one entry point against a decoder holding only an oversized prefix.
fn decode_oversized_prefix(entry: EntryPoint, limit: usize, declared: usize) -> (io::Error, usize) {
    let codec = LengthDelimitedFrameCodec::new(limit);
    // A fresh decoder per case, so neither entry point inherits state.
    let mut decoder = codec.decoder();
    // Only the length prefix is needed: tokio-util rejects the frame before any
    // payload has to arrive, so the close is the only thing that could differ.
    let prefix = u32::try_from(declared).expect("frame length should fit in u32");
    let mut buf = BytesMut::new();
    buf.extend_from_slice(&write_network_u32(prefix));

    let result = match entry {
        EntryPoint::Decode => decoder.decode(&mut buf),
        EntryPoint::DecodeEof => decoder.decode_eof(&mut buf),
    };
    let error = result.expect_err("an oversized length prefix must not decode");
    (error, buf.len())
}

/// Assert the shared classification contract for one entry point.
fn assert_is_oversized_framing_error(error: &io::Error, declared: usize, limit: usize) {
    assert_eq!(
        error.kind(),
        io::ErrorKind::InvalidData,
        "an oversized frame must stay an invalid-data error"
    );
    let classified = error
        .get_ref()
        .and_then(|inner| inner.downcast_ref::<CodecError>())
        .expect("the decoder should report a structured codec error");
    assert!(
        matches!(
            classified,
            CodecError::Framing(FramingError::OversizedFrame { size, max })
                if *size == declared && *max == limit
        ),
        "expected an oversized framing error for {declared} bytes, got {classified:?}"
    );
}

/// An oversized prefix is a framing failure on both entry points.
///
/// The EOF path meets the same inner `InvalidData` failure as `decode`, and the
/// wrapper must classify it rather than falling through to its plain
/// end-of-file error. Reporting `io` rather than `framing` for the same bytes
/// would apply the wrong policy.
#[rstest]
#[case::decode(EntryPoint::Decode)]
#[case::decode_eof(EntryPoint::DecodeEof)]
fn an_oversized_length_prefix_reports_a_framing_error(#[case] entry: EntryPoint) {
    let limit: usize = 128;
    let declared = limit.saturating_add(1);

    let (error, buffered) = decode_oversized_prefix(entry, limit, declared);

    assert_is_oversized_framing_error(&error, declared, limit);
    assert_eq!(
        buffered, 4,
        "{entry:?} must leave the prefix buffered so recovery sees zero consumption"
    );
}
