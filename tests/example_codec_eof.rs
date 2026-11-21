//! Structured end-of-file conditions for the bundled example codecs.
//!
//! The Hotline and `MySQL` adapters are the worked examples of custom framing,
//! so they must opt in to the same `on_eof` contract as the bundled
//! length-delimited decoder. A peer that closes mid-header or mid-payload has
//! to produce `EofError::MidHeader` or `EofError::MidFrame` from `decode_eof`,
//! leaving the partial bytes in the buffer for the connection to report.
//!
//! A malformed header is a framing error, not a truncation, and a close at a
//! frame boundary is a clean close. Both must stay outside the EOF taxonomy,
//! or a protocol would be told that data was lost when none was.
#![cfg(not(loom))]
#![forbid(unsafe_code)]

use std::io;

use bytes::{BufMut, Bytes, BytesMut};
use tokio_util::codec::{Decoder, Encoder};
use wireframe::codec::{
    CodecError,
    EofError,
    FrameCodec,
    examples::{HotlineFrame, HotlineFrameCodec, MysqlFrame, MysqlFrameCodec},
};

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

/// Encode one Hotline frame whose payload is `payload_len` bytes of `0x7`.
fn hotline_wire(payload_len: usize) -> BytesMut {
    let mut encoder = HotlineFrameCodec::new(64).encoder();
    let mut buffer = BytesMut::new();
    encoder
        .encode(
            HotlineFrame {
                transaction_id: 1,
                payload: Bytes::from(vec![7_u8; payload_len]),
            },
            &mut buffer,
        )
        .expect("encode a complete Hotline frame");
    buffer
}

/// Encode one `MySQL` frame whose payload is `payload_len` bytes of `0x7`.
fn mysql_wire(payload_len: usize) -> BytesMut {
    let mut encoder = MysqlFrameCodec::new(64).encoder();
    let mut buffer = BytesMut::new();
    encoder
        .encode(
            MysqlFrame {
                sequence_id: 1,
                payload: Bytes::from(vec![7_u8; payload_len]),
            },
            &mut buffer,
        )
        .expect("encode a complete MySQL frame");
    buffer
}

#[test]
fn hotline_decode_eof_reports_a_partial_header() {
    let mut decoder = HotlineFrameCodec::new(64).decoder();
    let mut buffer = BytesMut::from(&[0_u8; 7][..]);

    assert!(
        decoder
            .decode(&mut buffer)
            .expect("an incomplete header is not an error")
            .is_none()
    );
    let error = decoder
        .decode_eof(&mut buffer)
        .expect_err("a truncated header must be an error");
    assert_eq!(
        eof_condition(&error),
        Some(EofError::MidHeader {
            bytes_received: 7,
            header_size: 20,
        })
    );
    assert_eq!(buffer.len(), 7, "the partial header must not be consumed");
}

#[test]
fn hotline_decode_eof_reports_a_partial_payload() {
    let mut decoder = HotlineFrameCodec::new(64).decoder();
    let mut buffer = hotline_wire(8);
    buffer.truncate(20 + 3);

    assert!(
        decoder
            .decode(&mut buffer)
            .expect("an incomplete payload is not an error")
            .is_none()
    );
    let error = decoder
        .decode_eof(&mut buffer)
        .expect_err("a truncated payload must be an error");
    assert_eq!(
        eof_condition(&error),
        Some(EofError::MidFrame {
            bytes_received: 3,
            expected: 8,
        })
    );
    assert_eq!(
        buffer.len(),
        20 + 3,
        "the partial frame must not be consumed"
    );
}

#[test]
fn hotline_decode_eof_is_clean_at_a_frame_boundary() {
    let mut decoder = HotlineFrameCodec::new(64).decoder();
    let mut buffer = BytesMut::new();

    assert!(
        decoder
            .decode_eof(&mut buffer)
            .expect("a clean close is not an error")
            .is_none()
    );
}

#[test]
fn hotline_decode_eof_decodes_a_complete_frame() {
    let mut decoder = HotlineFrameCodec::new(64).decoder();
    let mut buffer = hotline_wire(5);

    let frame = decoder
        .decode_eof(&mut buffer)
        .expect("a complete frame decodes")
        .expect("the frame is present");
    assert_eq!(frame.payload, Bytes::from(vec![7_u8; 5]));
    assert!(buffer.is_empty(), "a complete frame is consumed");
}

#[test]
fn hotline_decode_eof_keeps_framing_errors() {
    let mut decoder = HotlineFrameCodec::new(64).decoder();
    let mut buffer = BytesMut::new();
    buffer.put_u32(1);
    // The declared frame length is not the payload plus the header.
    buffer.put_u32(1);
    buffer.put_u32(7);
    buffer.extend_from_slice(&[0_u8; 8]);

    let error = decoder
        .decode_eof(&mut buffer)
        .expect_err("a malformed header must fail");
    assert_eq!(error.kind(), io::ErrorKind::InvalidData);
    assert_eq!(
        eof_condition(&error),
        None,
        "a framing error must not be reported as a truncation"
    );
}

#[test]
fn mysql_decode_eof_reports_a_partial_header() {
    let mut decoder = MysqlFrameCodec::new(64).decoder();
    let mut buffer = BytesMut::from(&[0_u8; 3][..]);

    assert!(
        decoder
            .decode(&mut buffer)
            .expect("an incomplete header is not an error")
            .is_none()
    );
    let error = decoder
        .decode_eof(&mut buffer)
        .expect_err("a truncated header must be an error");
    assert_eq!(
        eof_condition(&error),
        Some(EofError::MidHeader {
            bytes_received: 3,
            header_size: 4,
        })
    );
    assert_eq!(buffer.len(), 3, "the partial header must not be consumed");
}

#[test]
fn mysql_decode_eof_reports_a_partial_payload() {
    let mut decoder = MysqlFrameCodec::new(64).decoder();
    let mut buffer = mysql_wire(8);
    buffer.truncate(4 + 5);

    assert!(
        decoder
            .decode(&mut buffer)
            .expect("an incomplete payload is not an error")
            .is_none()
    );
    let error = decoder
        .decode_eof(&mut buffer)
        .expect_err("a truncated payload must be an error");
    assert_eq!(
        eof_condition(&error),
        Some(EofError::MidFrame {
            bytes_received: 5,
            expected: 8,
        })
    );
    assert_eq!(
        buffer.len(),
        4 + 5,
        "the partial frame must not be consumed"
    );
}

#[test]
fn mysql_decode_eof_is_clean_at_a_frame_boundary() {
    let mut decoder = MysqlFrameCodec::new(64).decoder();
    let mut buffer = BytesMut::new();

    assert!(
        decoder
            .decode_eof(&mut buffer)
            .expect("a clean close is not an error")
            .is_none()
    );
}

#[test]
fn mysql_decode_eof_decodes_a_complete_frame() {
    let mut decoder = MysqlFrameCodec::new(64).decoder();
    let mut buffer = mysql_wire(5);

    let frame = decoder
        .decode_eof(&mut buffer)
        .expect("a complete frame decodes")
        .expect("the frame is present");
    assert_eq!(frame.payload, Bytes::from(vec![7_u8; 5]));
    assert!(buffer.is_empty(), "a complete frame is consumed");
}

#[test]
fn mysql_decode_eof_keeps_framing_errors() {
    // A codec with a three-byte limit rejects the four-byte header at once.
    let mut decoder = MysqlFrameCodec::new(3).decoder();
    let mut buffer = BytesMut::from(&[4_u8, 0, 0, 1][..]);

    let error = decoder
        .decode_eof(&mut buffer)
        .expect_err("an oversized header must fail");
    assert_eq!(error.kind(), io::ErrorKind::InvalidData);
    assert_eq!(
        eof_condition(&error),
        None,
        "a framing error must not be reported as a truncation"
    );
}
