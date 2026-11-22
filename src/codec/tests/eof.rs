//! Truncation classification for the length-delimited decoder.
//!
//! A peer may disconnect between the first and last byte of a frame. The
//! decoder must report that as a structured [`EofError`] while leaving the
//! partial frame untouched, so the connection can pass the exact bytes it
//! received to the protocol `on_eof` hook. These tests drive one decoder
//! through `decode` and then `decode_eof`, which is the sequence `FramedRead`
//! performs when the peer half-closes mid-frame.

use bytes::{Bytes, BytesMut};
use rstest::rstest;
use tokio_util::codec::{Decoder, Encoder};

use crate::{
    byte_order::write_network_u32,
    codec::{
        EofError,
        FrameCodec,
        LENGTH_HEADER_SIZE,
        LengthDelimitedDecoder,
        LengthDelimitedFrameCodec,
        eof_condition,
    },
};

/// Build a decoder for the default length-delimited codec.
fn decoder(max_frame_length: usize) -> LengthDelimitedDecoder {
    LengthDelimitedFrameCodec::new(max_frame_length).decoder()
}

/// Build a big-endian length prefix followed by `payload`.
fn frame_bytes(declared: usize, payload: &[u8]) -> BytesMut {
    let length_prefix = u32::try_from(declared).unwrap_or(u32::MAX);
    let mut buffer = BytesMut::from(&write_network_u32(length_prefix)[..]);
    buffer.extend_from_slice(payload);
    buffer
}

/// Decode and require that the buffer is still incomplete and untouched.
fn assert_incomplete(decoder: &mut LengthDelimitedDecoder, buffer: &mut BytesMut) {
    let before = buffer.clone();
    let frame = decoder
        .decode(buffer)
        .expect("an incomplete frame is not an error");
    assert!(frame.is_none(), "an incomplete frame must not decode");
    assert_eq!(
        &*buffer, &before,
        "an incomplete frame must stay in the read buffer"
    );
}

#[test]
fn partial_header_reports_the_bytes_received() {
    let mut decoder = decoder(128);
    let mut buffer = BytesMut::from(&[0x00_u8, 0x10][..]);

    assert_incomplete(&mut decoder, &mut buffer);

    let error = decoder
        .decode_eof(&mut buffer)
        .expect_err("truncated header must be an error");
    assert_eq!(
        eof_condition(&error),
        Some(EofError::MidHeader {
            bytes_received: 2,
            header_size: LENGTH_HEADER_SIZE,
        })
    );
    assert_eq!(buffer.len(), 2, "the partial header must not be consumed");
}

/// Every prefix short of the declared payload is reported with its exact size.
///
/// A complete length prefix with no payload bytes at all is still a truncated
/// frame rather than a truncated header, so it is the zero-length case here.
#[rstest]
#[case::header_only(vec![])]
#[case::one_byte(vec![1_u8])]
#[case::three_bytes(vec![1_u8, 2, 3])]
#[case::seven_bytes(vec![1_u8, 2, 3, 4, 5, 6, 7])]
fn partial_payload_reports_the_payload_bytes_received(#[case] payload: Vec<u8>) {
    let mut decoder = decoder(128);
    let mut buffer = frame_bytes(8, &payload);
    let received = payload.len();

    assert_incomplete(&mut decoder, &mut buffer);

    let error = decoder
        .decode_eof(&mut buffer)
        .expect_err("a truncated payload must be an error");
    assert_eq!(
        eof_condition(&error),
        Some(EofError::MidFrame {
            bytes_received: received,
            expected: 8,
        })
    );
}

#[test]
fn complete_frame_is_decoded_before_a_truncated_successor() {
    let codec = LengthDelimitedFrameCodec::new(128);
    let mut encoder = codec.encoder();
    let mut decoder = codec.decoder();
    let mut buffer = BytesMut::new();
    encoder
        .encode(Bytes::from_static(b"first"), &mut buffer)
        .expect("encode the first frame");
    let second_start = buffer.len();
    encoder
        .encode(Bytes::from_static(b"second"), &mut buffer)
        .expect("encode the second frame");
    // Keep the second frame's whole length prefix and three payload bytes, so
    // the truncation is mid-payload rather than mid-header.
    buffer.truncate(second_start + LENGTH_HEADER_SIZE + 3);

    let first = decoder
        .decode(&mut buffer)
        .expect("the complete frame decodes")
        .expect("a complete frame is present");
    assert_eq!(first.as_ref(), b"first");

    // The remaining buffer is the second frame's prefix plus three payload
    // bytes; the hook receives all of it, because none of it is a frame.
    let partial = buffer.clone();
    assert_incomplete(&mut decoder, &mut buffer);

    let error = decoder
        .decode_eof(&mut buffer)
        .expect_err("the truncated successor must be an error");
    assert_eq!(
        eof_condition(&error),
        Some(EofError::MidFrame {
            bytes_received: 3,
            expected: 6,
        })
    );
    assert_eq!(
        &*buffer, &partial,
        "the exact unconsumed bytes must remain available to on_eof"
    );
}

#[test]
fn oversized_declared_length_is_a_framing_error_not_truncation() {
    let mut decoder = decoder(64);
    let mut buffer = frame_bytes(65, &[]);

    let decode_error = decoder
        .decode(&mut buffer)
        .expect_err("the declared length exceeds the maximum");
    assert_eq!(decode_error.kind(), std::io::ErrorKind::InvalidData);
    assert!(
        eof_condition(&decode_error).is_none(),
        "an oversized frame is not an EOF condition"
    );

    let eof_error = decoder
        .decode_eof(&mut buffer)
        .expect_err("the declared length still exceeds the maximum");
    assert_eq!(eof_error.kind(), std::io::ErrorKind::InvalidData);
    assert!(
        eof_condition(&eof_error).is_none(),
        "an oversized frame must not run the on_eof hook"
    );
}

#[test]
fn plain_io_error_is_not_an_eof_condition() {
    let error = std::io::Error::other("connection reset");
    assert!(eof_condition(&error).is_none());
}
