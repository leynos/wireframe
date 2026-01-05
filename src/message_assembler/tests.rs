//! Unit tests for message assembler header parsing.

use std::io;

use bytes::{Buf, BufMut, BytesMut};

use super::{
    ContinuationFrameHeader,
    FirstFrameHeader,
    FrameHeader,
    FrameSequence,
    MessageAssembler,
    MessageKey,
    ParsedFrameHeader,
};

struct TestAssembler;

impl MessageAssembler for TestAssembler {
    fn parse_frame_header(&self, payload: &[u8]) -> Result<ParsedFrameHeader, io::Error> {
        let mut buf = payload;
        let initial = buf.remaining();

        let kind = take_u8(&mut buf)?;
        let flags = take_u8(&mut buf)?;
        let message_key = MessageKey::from(take_u64(&mut buf)?);

        let header = match kind {
            0x01 => {
                let metadata_len = usize::from(take_u16(&mut buf)?);
                let body_len = usize::try_from(take_u32(&mut buf)?)
                    .map_err(|_| invalid_data("body length too large"))?;
                let total_body_len = if flags & 0b10 == 0b10 {
                    Some(
                        usize::try_from(take_u32(&mut buf)?)
                            .map_err(|_| invalid_data("total length too large"))?,
                    )
                } else {
                    None
                };

                FrameHeader::First(FirstFrameHeader {
                    message_key,
                    metadata_len,
                    body_len,
                    total_body_len,
                    is_last: flags & 0b1 == 0b1,
                })
            }
            0x02 => {
                let body_len = usize::try_from(take_u32(&mut buf)?)
                    .map_err(|_| invalid_data("body length too large"))?;
                let sequence = if flags & 0b10 == 0b10 {
                    Some(FrameSequence::from(take_u32(&mut buf)?))
                } else {
                    None
                };

                FrameHeader::Continuation(ContinuationFrameHeader {
                    message_key,
                    sequence,
                    body_len,
                    is_last: flags & 0b1 == 0b1,
                })
            }
            _ => return Err(invalid_data("unknown header kind")),
        };

        let header_len = initial - buf.remaining();
        Ok(ParsedFrameHeader::new(header, header_len))
    }
}

#[test]
fn parse_frame_headers() {
    let cases = build_test_cases();

    for case in cases {
        let payload = (case.build_payload)();
        let parsed = parse_header(&payload);
        assert_eq!(parsed.header(), &case.expected_header, "case: {}", case.name);
        assert_eq!(parsed.header_len(), payload.len(), "case: {}", case.name);
    }
}

#[test]
fn short_header_errors() {
    let payload = vec![0x01];
    let err = TestAssembler
        .parse_frame_header(&payload)
        .expect_err("expected error");

    assert_eq!(err.kind(), io::ErrorKind::InvalidData);
}

#[test]
fn unknown_header_kind_errors() {
    let payload = vec![0xff, 0x00, 0x00];
    let err = TestAssembler
        .parse_frame_header(&payload)
        .expect_err("expected error");

    assert_eq!(err.kind(), io::ErrorKind::InvalidData);
}

fn take_u8(buf: &mut &[u8]) -> Result<u8, io::Error> {
    ensure_remaining(buf, 1)?;
    Ok(buf.get_u8())
}

fn take_u16(buf: &mut &[u8]) -> Result<u16, io::Error> {
    ensure_remaining(buf, 2)?;
    Ok(buf.get_u16())
}

fn take_u32(buf: &mut &[u8]) -> Result<u32, io::Error> {
    ensure_remaining(buf, 4)?;
    Ok(buf.get_u32())
}

fn take_u64(buf: &mut &[u8]) -> Result<u64, io::Error> {
    ensure_remaining(buf, 8)?;
    Ok(buf.get_u64())
}

fn ensure_remaining(buf: &mut &[u8], needed: usize) -> Result<(), io::Error> {
    if buf.remaining() < needed {
        return Err(invalid_data("header too short"));
    }
    Ok(())
}

fn invalid_data(message: &'static str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, message)
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

fn build_test_cases() -> Vec<HeaderCase> {
    vec![
        HeaderCase {
            name: "first frame without total",
            build_payload: Box::new(|| {
                build_first_header_payload(FirstHeaderSpec {
                    flags: 0b0,
                    message_key: 9,
                    metadata_len: 2,
                    body_len: 12,
                    total_body_len: None,
                })
            }),
            expected_header: FrameHeader::First(FirstFrameHeader {
                message_key: MessageKey(9),
                metadata_len: 2,
                body_len: 12,
                total_body_len: None,
                is_last: false,
            }),
        },
        HeaderCase {
            name: "first frame with total and last",
            build_payload: Box::new(|| {
                build_first_header_payload(FirstHeaderSpec {
                    flags: 0b11,
                    message_key: 42,
                    metadata_len: 0,
                    body_len: 8,
                    total_body_len: Some(64),
                })
            }),
            expected_header: FrameHeader::First(FirstFrameHeader {
                message_key: MessageKey(42),
                metadata_len: 0,
                body_len: 8,
                total_body_len: Some(64),
                is_last: true,
            }),
        },
        HeaderCase {
            name: "continuation frame with sequence",
            build_payload: Box::new(|| {
                build_continuation_header_payload(ContinuationHeaderSpec {
                    flags: 0b10,
                    message_key: 7,
                    body_len: 16,
                    sequence: Some(3),
                })
            }),
            expected_header: FrameHeader::Continuation(ContinuationFrameHeader {
                message_key: MessageKey(7),
                sequence: Some(FrameSequence(3)),
                body_len: 16,
                is_last: false,
            }),
        },
        HeaderCase {
            name: "continuation frame without sequence",
            build_payload: Box::new(|| {
                build_continuation_header_payload(ContinuationHeaderSpec {
                    flags: 0b1,
                    message_key: 11,
                    body_len: 5,
                    sequence: None,
                })
            }),
            expected_header: FrameHeader::Continuation(ContinuationFrameHeader {
                message_key: MessageKey(11),
                sequence: None,
                body_len: 5,
                is_last: true,
            }),
        },
    ]
}

struct HeaderCase {
    name: &'static str,
    build_payload: Box<dyn Fn() -> Vec<u8>>,
    expected_header: FrameHeader,
}
