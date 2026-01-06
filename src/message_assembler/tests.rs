//! Unit tests for message assembler header parsing.

use std::io;

use bytes::{BufMut, BytesMut};
use rstest::rstest;

use super::{
    ContinuationFrameHeader,
    FirstFrameHeader,
    FrameHeader,
    FrameSequence,
    MessageAssembler,
    MessageKey,
    ParsedFrameHeader,
};
use crate::test_helpers::TestAssembler;

#[rstest]
#[case::first_frame_without_total(
    "first frame without total",
    build_first_header_payload(FirstHeaderSpec {
        flags: 0b0,
        message_key: 9,
        metadata_len: 2,
        body_len: 12,
        total_body_len: None,
    }),
    FrameHeader::First(FirstFrameHeader {
        message_key: MessageKey(9),
        metadata_len: 2,
        body_len: 12,
        total_body_len: None,
        is_last: false,
    }),
)]
#[case::first_frame_with_total_and_last(
    "first frame with total and last",
    build_first_header_payload(FirstHeaderSpec {
        flags: 0b11,
        message_key: 42,
        metadata_len: 0,
        body_len: 8,
        total_body_len: Some(64),
    }),
    FrameHeader::First(FirstFrameHeader {
        message_key: MessageKey(42),
        metadata_len: 0,
        body_len: 8,
        total_body_len: Some(64),
        is_last: true,
    }),
)]
#[case::continuation_frame_with_sequence(
    "continuation frame with sequence",
    build_continuation_header_payload(ContinuationHeaderSpec {
        flags: 0b10,
        message_key: 7,
        body_len: 16,
        sequence: Some(3),
    }),
    FrameHeader::Continuation(ContinuationFrameHeader {
        message_key: MessageKey(7),
        sequence: Some(FrameSequence(3)),
        body_len: 16,
        is_last: false,
    }),
)]
#[case::continuation_frame_without_sequence(
    "continuation frame without sequence",
    build_continuation_header_payload(ContinuationHeaderSpec {
        flags: 0b1,
        message_key: 11,
        body_len: 5,
        sequence: None,
    }),
    FrameHeader::Continuation(ContinuationFrameHeader {
        message_key: MessageKey(11),
        sequence: None,
        body_len: 5,
        is_last: true,
    }),
)]
fn parse_frame_headers(
    #[case] case_name: &'static str,
    #[case] payload: Vec<u8>,
    #[case] expected_header: FrameHeader,
) {
    let parsed = parse_header(&payload);
    assert_eq!(parsed.header(), &expected_header, "case: {case_name}");
    assert_eq!(
        parsed.header_len(),
        payload.len(),
        "case (header-only payload): {case_name}"
    );

    let mut payload_with_body = payload.clone();
    payload_with_body.extend_from_slice(&[0xaa, 0xbb, 0xcc]);

    let parsed_with_body = parse_header(&payload_with_body);
    assert_eq!(
        parsed_with_body.header(),
        &expected_header,
        "case (with body bytes): {case_name}"
    );
    assert_eq!(
        parsed_with_body.header_len(),
        payload.len(),
        "case (with body bytes, header_len mismatch): {case_name}"
    );
    assert!(
        parsed_with_body.header_len() < payload_with_body.len(),
        "case (with body bytes, header_len not less than payload.len()): {case_name}"
    );
}

#[test]
fn short_header_errors() {
    let payload = vec![0x01];
    let err = TestAssembler
        .parse_frame_header(&payload)
        .expect_err("expected error");

    assert_eq!(err.kind(), io::ErrorKind::InvalidData);
    assert_eq!(err.to_string(), "header too short");
}

#[test]
fn unknown_header_kind_errors() {
    let mut bytes = BytesMut::new();
    bytes.put_u8(0xff);
    bytes.put_u8(0x00);
    bytes.put_u64(0);
    let payload = bytes.to_vec();
    let err = TestAssembler
        .parse_frame_header(&payload)
        .expect_err("expected error");

    assert_eq!(err.kind(), io::ErrorKind::InvalidData);
    assert_eq!(err.to_string(), "unknown header kind");
}

#[derive(Clone, Copy)]
struct FirstHeaderSpec {
    flags: u8,
    message_key: u64,
    metadata_len: u16,
    body_len: u32,
    total_body_len: Option<u32>,
}

#[derive(Clone, Copy)]
struct ContinuationHeaderSpec {
    flags: u8,
    message_key: u64,
    body_len: u32,
    sequence: Option<u32>,
}

fn build_first_header_payload(spec: FirstHeaderSpec) -> Vec<u8> {
    let mut bytes = BytesMut::new();
    bytes.put_u8(0x01);
    bytes.put_u8(spec.flags);
    bytes.put_u64(spec.message_key);
    bytes.put_u16(spec.metadata_len);
    bytes.put_u32(spec.body_len);
    if let Some(total_body_len) = spec.total_body_len {
        bytes.put_u32(total_body_len);
    }
    bytes.to_vec()
}

fn build_continuation_header_payload(spec: ContinuationHeaderSpec) -> Vec<u8> {
    let mut bytes = BytesMut::new();
    bytes.put_u8(0x02);
    bytes.put_u8(spec.flags);
    bytes.put_u64(spec.message_key);
    bytes.put_u32(spec.body_len);
    if let Some(sequence) = spec.sequence {
        bytes.put_u32(sequence);
    }
    bytes.to_vec()
}

fn parse_header(payload: &[u8]) -> ParsedFrameHeader {
    TestAssembler
        .parse_frame_header(payload)
        .expect("header parse")
}
