//! RESP parsing functions.

use std::{io, str};

use bytes::BytesMut;

use super::frame::RespFrame;

/// Maximum number of elements permitted in a RESP array.
///
/// This limit prevents allocation-based denial-of-service attacks where a
/// malicious client sends a large array count to trigger excessive memory
/// allocation before any payload bytes are validated.
const MAX_ARRAY_ELEMENTS: usize = 1024;

/// Maximum recursion depth for nested RESP arrays.
///
/// This limit prevents stack overflow attacks where a malicious client sends
/// deeply nested arrays (e.g., `*1\r\n*1\r\n*1\r\n...`) to exhaust the stack.
const MAX_RECURSION_DEPTH: usize = 64;

/// Parse a complete RESP frame from the buffer.
pub fn parse_frame(
    buf: &BytesMut,
    max_frame_length: usize,
) -> Result<Option<(RespFrame, usize)>, io::Error> {
    parse_frame_at(buf, 0, max_frame_length, 0)
}

/// Parse a line and convert to UTF-8.
fn parse_text_line<'a>(
    buf: &'a BytesMut,
    start: usize,
    max_frame_length: usize,
    error_msg: &str,
) -> Result<Option<(&'a str, usize)>, io::Error> {
    let Some((line, next)) = parse_line(buf, start + 1, max_frame_length)? else {
        return Ok(None);
    };
    let text =
        str::from_utf8(line).map_err(|_| io::Error::new(io::ErrorKind::InvalidData, error_msg))?;
    Ok(Some((text, next)))
}

fn parse_simple_string(
    buf: &BytesMut,
    start: usize,
    max_frame_length: usize,
) -> Result<Option<(RespFrame, usize)>, io::Error> {
    let Some((text, next)) =
        parse_text_line(buf, start, max_frame_length, "invalid simple string")?
    else {
        return Ok(None);
    };
    let frame = RespFrame::SimpleString(text.to_string());
    Ok(Some((frame, next - start)))
}

fn parse_integer(
    buf: &BytesMut,
    start: usize,
    max_frame_length: usize,
) -> Result<Option<(RespFrame, usize)>, io::Error> {
    let Some((text, next)) = parse_text_line(buf, start, max_frame_length, "invalid integer")?
    else {
        return Ok(None);
    };
    let value = text
        .parse::<i64>()
        .map_err(|_| io::Error::new(io::ErrorKind::InvalidData, "invalid integer"))?;
    Ok(Some((RespFrame::Integer(value), next - start)))
}

enum BulkLength {
    Null,
    Sized(usize),
}

#[derive(Clone, Copy, Debug)]
struct BulkPayloadSpec {
    start: usize,
    payload_start: usize,
    len: usize,
    max_frame_length: usize,
}

fn parse_bulk_string(
    buf: &BytesMut,
    start: usize,
    max_frame_length: usize,
) -> Result<Option<(RespFrame, usize)>, io::Error> {
    let Some((length, next)) = parse_bulk_length(buf, start, max_frame_length)? else {
        return Ok(None);
    };
    match length {
        BulkLength::Null => Ok(Some((RespFrame::BulkString(None), next - start))),
        BulkLength::Sized(len) => {
            let spec = BulkPayloadSpec {
                start,
                payload_start: next,
                len,
                max_frame_length,
            };
            parse_bulk_payload(buf, spec)
        }
    }
}

fn parse_bulk_length(
    buf: &BytesMut,
    start: usize,
    max_frame_length: usize,
) -> Result<Option<(BulkLength, usize)>, io::Error> {
    let Some((line, next)) = parse_line(buf, start + 1, max_frame_length)? else {
        return Ok(None);
    };
    let text = str::from_utf8(line)
        .map_err(|_| io::Error::new(io::ErrorKind::InvalidData, "invalid bulk length"))?;
    let len = text
        .parse::<i64>()
        .map_err(|_| io::Error::new(io::ErrorKind::InvalidData, "invalid bulk length"))?;
    if len < -1 {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "invalid bulk length",
        ));
    }
    if len == -1 {
        return Ok(Some((BulkLength::Null, next)));
    }

    let len = usize::try_from(len)
        .map_err(|_| io::Error::new(io::ErrorKind::InvalidData, "bulk length too large"))?;
    Ok(Some((BulkLength::Sized(len), next)))
}

fn parse_bulk_payload(
    buf: &BytesMut,
    spec: BulkPayloadSpec,
) -> Result<Option<(RespFrame, usize)>, io::Error> {
    if spec.len > spec.max_frame_length {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "bulk length exceeds max frame",
        ));
    }
    let end = spec
        .payload_start
        .checked_add(spec.len)
        .and_then(|value| value.checked_add(2))
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, "bulk length too large"))?;
    if buf.len() < end {
        return Ok(None);
    }

    let data = buf
        .get(spec.payload_start..spec.payload_start + spec.len)
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, "invalid bulk range"))?;
    validate_bulk_terminator(buf, spec.payload_start + spec.len)?;

    Ok(Some((
        RespFrame::BulkString(Some(data.to_vec())),
        end - spec.start,
    )))
}

fn validate_bulk_terminator(buf: &BytesMut, cursor: usize) -> Result<(), io::Error> {
    let cr = buf
        .get(cursor)
        .copied()
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, "missing terminator"))?;
    let lf = buf
        .get(cursor + 1)
        .copied()
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, "missing terminator"))?;
    if cr != b'\r' || lf != b'\n' {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "bulk string missing terminator",
        ));
    }
    Ok(())
}

fn parse_array(
    buf: &BytesMut,
    start: usize,
    max_frame_length: usize,
    depth: usize,
) -> Result<Option<(RespFrame, usize)>, io::Error> {
    let Some((line, mut cursor)) = parse_line(buf, start + 1, max_frame_length)? else {
        return Ok(None);
    };
    let text = str::from_utf8(line)
        .map_err(|_| io::Error::new(io::ErrorKind::InvalidData, "invalid array length"))?;
    let count = text
        .parse::<i64>()
        .map_err(|_| io::Error::new(io::ErrorKind::InvalidData, "invalid array length"))?;
    if count < 0 {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "negative array length",
        ));
    }
    let count = usize::try_from(count)
        .map_err(|_| io::Error::new(io::ErrorKind::InvalidData, "array too large"))?;
    // Limit element count to prevent allocation-based DoS. The cumulative
    // byte validation at the end of this function enforces the actual
    // byte-budget constraint.
    if count > MAX_ARRAY_ELEMENTS {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "array element count exceeds limit",
        ));
    }
    let mut frames = Vec::with_capacity(count);
    for _ in 0..count {
        let Some((frame, consumed)) = parse_frame_at(buf, cursor, max_frame_length, depth + 1)?
        else {
            return Ok(None);
        };
        cursor = cursor
            .checked_add(consumed)
            .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, "array too large"))?;
        frames.push(frame);
    }
    let consumed = cursor
        .checked_sub(start)
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, "array too large"))?;
    if consumed > max_frame_length {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "frame too large",
        ));
    }
    Ok(Some((RespFrame::Array(frames), consumed)))
}

fn parse_frame_at(
    buf: &BytesMut,
    start: usize,
    max_frame_length: usize,
    depth: usize,
) -> Result<Option<(RespFrame, usize)>, io::Error> {
    if depth > MAX_RECURSION_DEPTH {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "maximum recursion depth exceeded",
        ));
    }
    let Some(prefix) = buf.get(start).copied() else {
        return Ok(None);
    };
    match prefix {
        b'+' => parse_simple_string(buf, start, max_frame_length),
        b':' => parse_integer(buf, start, max_frame_length),
        b'$' => parse_bulk_string(buf, start, max_frame_length),
        b'*' => parse_array(buf, start, max_frame_length, depth),
        _ => Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "unsupported RESP prefix",
        )),
    }
}

fn parse_line(
    buf: &BytesMut,
    start: usize,
    max_len: usize,
) -> Result<Option<(&[u8], usize)>, io::Error> {
    let mut index = start;
    while let Some(byte) = buf.get(index).copied() {
        if index.saturating_sub(start) >= max_len {
            return Err(io::Error::new(io::ErrorKind::InvalidData, "line too long"));
        }
        if byte == b'\r' {
            let Some(next) = buf.get(index + 1).copied() else {
                return Ok(None);
            };
            if next == b'\n' {
                let line = buf
                    .get(start..index)
                    .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, "line bounds"))?;
                return Ok(Some((line, index + 2)));
            }
        }
        index = index
            .checked_add(1)
            .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, "line too long"))?;
    }
    Ok(None)
}
