//! Oversized length-prefix classification for the length-delimited codec.
//!
//! Tokio-util rejects a frame whose declared length exceeds the codec limit as
//! soon as the prefix is complete. Both entry points must report that failure
//! as the same structured framing error: the recovery hook selects its policy
//! from the error type, so a plain `io` error would apply the wrong policy to
//! an oversized frame that arrives before a close.

use std::io;

use bytes::BytesMut;

use super::*;

#[test]
fn decoding_an_oversized_length_prefix_reports_a_framing_error() {
    let limit = 128;
    let codec = LengthDelimitedFrameCodec::new(limit);
    let mut decoder = codec.decoder();
    // Only the length prefix is needed: tokio-util rejects the frame before
    // any payload has to arrive.
    let declared = limit.saturating_add(1);
    let prefix = u32::try_from(declared).expect("frame length should fit in u32");
    let mut buf = BytesMut::new();
    buf.extend_from_slice(&write_network_u32(prefix));

    let err = decoder
        .decode(&mut buf)
        .expect_err("expected decode to fail for an oversized length prefix");

    assert_eq!(
        err.kind(),
        io::ErrorKind::InvalidData,
        "an oversized frame should remain an invalid-data error"
    );
    assert_eq!(
        buf.len(),
        4,
        "the rejected frame must stay buffered so recovery sees zero consumption"
    );
    let classified = err
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

/// An oversized length prefix is still a framing failure when the peer closes.
///
/// The EOF path meets the same inner `InvalidData` failure as `decode`, and the
/// wrapper must classify it instead of falling through to its plain
/// end-of-file error. Reporting `io` rather than `framing` for the same bytes
/// would apply the wrong policy.
#[test]
fn decoding_an_oversized_length_prefix_at_eof_reports_a_framing_error() {
    let limit = 128;
    let codec = LengthDelimitedFrameCodec::new(limit);
    let mut decoder = codec.decoder();
    // Only the prefix is buffered: the frame is rejected before any payload
    // has to arrive, and the close is what ends the connection.
    let declared = limit.saturating_add(1);
    let prefix = u32::try_from(declared).expect("frame length should fit in u32");
    let mut buf = BytesMut::new();
    buf.extend_from_slice(&write_network_u32(prefix));

    let err = decoder
        .decode_eof(&mut buf)
        .expect_err("expected decode_eof to reject an oversized length prefix");

    assert_eq!(
        err.kind(),
        io::ErrorKind::InvalidData,
        "an oversized frame at EOF must stay an invalid-data error"
    );
    assert_eq!(
        buf.len(),
        4,
        "the rejected prefix must stay buffered so recovery sees zero consumption"
    );
    let classified = err
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
