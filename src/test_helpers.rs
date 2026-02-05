#![cfg(any(test, feature = "test-helpers"))]
//! Test-only helpers for shared test utilities.

use std::io;

use bytes::Buf;

use crate::message_assembler::{
    ContinuationFrameHeader,
    FirstFrameHeader,
    FrameHeader,
    FrameSequence,
    MessageAssembler,
    MessageKey,
    ParsedFrameHeader,
};

pub mod frame_codec;

pub use frame_codec::{TestAdapter, TestCodec, TestFrame};

/// Test-friendly message assembler implementation that shares parsing logic.
#[derive(Clone, Copy, Debug, Default)]
pub struct TestAssembler;

impl MessageAssembler for TestAssembler {
    fn parse_frame_header(&self, payload: &[u8]) -> Result<ParsedFrameHeader, io::Error> {
        parse_frame_header(payload)
    }
}

/// Parse a protocol-specific frame header for tests.
///
/// # Errors
///
/// Returns an error if the payload is too short or contains an invalid header.
pub fn parse_frame_header(payload: &[u8]) -> Result<ParsedFrameHeader, io::Error> {
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
