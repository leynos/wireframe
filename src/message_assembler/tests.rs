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
fn parse_first_frame_header_without_total() {
    let mut bytes = BytesMut::new();
    bytes.put_u8(0x01); // kind
    bytes.put_u8(0b0); // flags
    bytes.put_u64(9);
    bytes.put_u16(2);
    bytes.put_u32(12);
    let payload = bytes.to_vec();

    let parsed = TestAssembler
        .parse_frame_header(&payload)
        .expect("header parse");

    let FrameHeader::First(header) = parsed.header() else {
        panic!("expected first frame header");
    };

    assert_eq!(header.message_key, MessageKey(9));
    assert_eq!(header.metadata_len, 2);
    assert_eq!(header.body_len, 12);
    assert_eq!(header.total_body_len, None);
    assert!(!header.is_last);
    assert_eq!(parsed.header_len(), payload.len());
}

#[test]
fn parse_first_frame_header_with_total_and_last() {
    let mut bytes = BytesMut::new();
    bytes.put_u8(0x01);
    bytes.put_u8(0b11); // last + total
    bytes.put_u64(42);
    bytes.put_u16(0);
    bytes.put_u32(8);
    bytes.put_u32(64);
    let payload = bytes.to_vec();

    let parsed = TestAssembler
        .parse_frame_header(&payload)
        .expect("header parse");

    let FrameHeader::First(header) = parsed.header() else {
        panic!("expected first frame header");
    };

    assert_eq!(header.message_key, MessageKey(42));
    assert_eq!(header.metadata_len, 0);
    assert_eq!(header.body_len, 8);
    assert_eq!(header.total_body_len, Some(64));
    assert!(header.is_last);
    assert_eq!(parsed.header_len(), payload.len());
}

#[test]
fn parse_continuation_header_with_sequence() {
    let mut bytes = BytesMut::new();
    bytes.put_u8(0x02);
    bytes.put_u8(0b10); // sequence present
    bytes.put_u64(7);
    bytes.put_u32(16);
    bytes.put_u32(3);
    let payload = bytes.to_vec();

    let parsed = TestAssembler
        .parse_frame_header(&payload)
        .expect("header parse");

    let FrameHeader::Continuation(header) = parsed.header() else {
        panic!("expected continuation header");
    };

    assert_eq!(header.message_key, MessageKey(7));
    assert_eq!(header.body_len, 16);
    assert_eq!(header.sequence, Some(FrameSequence(3)));
    assert!(!header.is_last);
}

#[test]
fn parse_continuation_header_without_sequence() {
    let mut bytes = BytesMut::new();
    bytes.put_u8(0x02);
    bytes.put_u8(0b1); // last, no sequence
    bytes.put_u64(11);
    bytes.put_u32(5);
    let payload = bytes.to_vec();

    let parsed = TestAssembler
        .parse_frame_header(&payload)
        .expect("header parse");

    let FrameHeader::Continuation(header) = parsed.header() else {
        panic!("expected continuation header");
    };

    assert_eq!(header.message_key, MessageKey(11));
    assert_eq!(header.body_len, 5);
    assert_eq!(header.sequence, None);
    assert!(header.is_last);
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
