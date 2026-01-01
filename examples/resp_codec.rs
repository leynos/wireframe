#![cfg(feature = "examples")]
//! Example `RESP` framing codec wired into a `WireframeApp`.
//!
//! This is a minimal RESP parser/encoder that supports a subset of types:
//! - Simple strings (`+OK`)
//! - Integers (`:1`)
//! - Bulk strings (`$3\r\nfoo`)
//! - Arrays (`*2\r\n$3\r\nfoo\r\n:1`)
//!
//! Wireframe payloads are carried in bulk strings. Other frame types are
//! decoded for completeness but do not expose a payload for routing.

use std::{io, str};

use bytes::{Buf, BufMut, BytesMut};

/// Maximum number of elements permitted in a RESP array.
///
/// This limit prevents allocation-based denial-of-service attacks where a
/// malicious client sends a large array count to trigger excessive memory
/// allocation before any payload bytes are validated.
const MAX_ARRAY_ELEMENTS: usize = 1024;
use tokio_util::codec::{Decoder, Encoder};
use wireframe::{
    BincodeSerializer,
    app::{Envelope, WireframeApp},
    codec::FrameCodec,
};

#[derive(Clone, Debug, PartialEq, Eq)]
enum RespFrame {
    SimpleString(String),
    Integer(i64),
    BulkString(Option<Vec<u8>>),
    Array(Vec<RespFrame>),
}

#[derive(Clone, Debug)]
struct RespFrameCodec {
    max_frame_length: usize,
}

impl RespFrameCodec {
    fn new(max_frame_length: usize) -> Self { Self { max_frame_length } }
}

#[derive(Clone, Debug)]
struct RespAdapter {
    max_frame_length: usize,
}

impl RespAdapter {
    fn new(max_frame_length: usize) -> Self { Self { max_frame_length } }
}

impl Decoder for RespAdapter {
    type Item = RespFrame;
    type Error = io::Error;

    fn decode(&mut self, src: &mut BytesMut) -> Result<Option<Self::Item>, Self::Error> {
        let Some((frame, consumed)) = parse_frame(src, self.max_frame_length)? else {
            return Ok(None);
        };
        if consumed > self.max_frame_length {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "frame too large",
            ));
        }
        src.advance(consumed);
        Ok(Some(frame))
    }
}

impl Encoder<RespFrame> for RespAdapter {
    type Error = io::Error;

    fn encode(&mut self, item: RespFrame, dst: &mut BytesMut) -> Result<(), Self::Error> {
        let len = encoded_len(&item)?;
        if len > self.max_frame_length {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "frame too large",
            ));
        }
        dst.reserve(len);
        encode_frame(&item, dst)
    }
}

impl FrameCodec for RespFrameCodec {
    type Frame = RespFrame;
    type Decoder = RespAdapter;
    type Encoder = RespAdapter;

    fn decoder(&self) -> Self::Decoder { RespAdapter::new(self.max_frame_length) }

    fn encoder(&self) -> Self::Encoder { RespAdapter::new(self.max_frame_length) }

    fn frame_payload(frame: &Self::Frame) -> &[u8] {
        match frame {
            RespFrame::BulkString(Some(payload)) => payload.as_slice(),
            _ => &[],
        }
    }

    fn wrap_payload(payload: Vec<u8>) -> Self::Frame { RespFrame::BulkString(Some(payload)) }

    fn max_frame_length(&self) -> usize { self.max_frame_length }
}

fn parse_frame(
    buf: &BytesMut,
    max_frame_length: usize,
) -> Result<Option<(RespFrame, usize)>, io::Error> {
    parse_frame_at(buf, 0, max_frame_length)
}

fn parse_simple_string(
    buf: &BytesMut,
    start: usize,
    max_frame_length: usize,
) -> Result<Option<(RespFrame, usize)>, io::Error> {
    let Some((line, next)) = parse_line(buf, start + 1, max_frame_length)? else {
        return Ok(None);
    };
    let text = str::from_utf8(line)
        .map_err(|_| io::Error::new(io::ErrorKind::InvalidData, "invalid simple string"))?;
    let frame = RespFrame::SimpleString(text.to_string());
    Ok(Some((frame, next - start)))
}

fn parse_integer(
    buf: &BytesMut,
    start: usize,
    max_frame_length: usize,
) -> Result<Option<(RespFrame, usize)>, io::Error> {
    let Some((line, next)) = parse_line(buf, start + 1, max_frame_length)? else {
        return Ok(None);
    };
    let text = str::from_utf8(line)
        .map_err(|_| io::Error::new(io::ErrorKind::InvalidData, "invalid integer"))?;
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
        let Some((frame, consumed)) = parse_frame_at(buf, cursor, max_frame_length)? else {
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
) -> Result<Option<(RespFrame, usize)>, io::Error> {
    let Some(prefix) = buf.get(start).copied() else {
        return Ok(None);
    };
    match prefix {
        b'+' => parse_simple_string(buf, start, max_frame_length),
        b':' => parse_integer(buf, start, max_frame_length),
        b'$' => parse_bulk_string(buf, start, max_frame_length),
        b'*' => parse_array(buf, start, max_frame_length),
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

fn digits_len_usize(mut value: usize) -> usize {
    let mut digits = 1;
    while value >= 10 {
        value /= 10;
        digits += 1;
    }
    digits
}

fn digits_len_i64(value: i64) -> usize {
    let mut digits = 1;
    let mut cursor = value;
    while cursor <= -10 || cursor >= 10 {
        cursor /= 10;
        digits += 1;
    }
    if value < 0 { digits + 1 } else { digits }
}

fn encoded_len(frame: &RespFrame) -> Result<usize, io::Error> {
    match frame {
        RespFrame::SimpleString(text) => checked_add(1, checked_add(text.len(), 2)?),
        RespFrame::Integer(value) => {
            let digits = digits_len_i64(*value);
            checked_add(1, checked_add(digits, 2)?)
        }
        RespFrame::BulkString(None) => Ok(5),
        RespFrame::BulkString(Some(data)) => {
            let len_digits = digits_len_usize(data.len());
            let head = checked_add(1, checked_add(len_digits, 2)?)?;
            let body = checked_add(data.len(), 2)?;
            checked_add(head, body)
        }
        RespFrame::Array(items) => {
            let count_digits = digits_len_usize(items.len());
            let mut total = checked_add(1, checked_add(count_digits, 2)?)?;
            for item in items {
                total = checked_add(total, encoded_len(item)?)?;
            }
            Ok(total)
        }
    }
}

fn encode_frame(frame: &RespFrame, dst: &mut BytesMut) -> Result<(), io::Error> {
    match frame {
        RespFrame::SimpleString(text) => {
            dst.put_u8(b'+');
            dst.extend_from_slice(text.as_bytes());
            dst.extend_from_slice(b"\r\n");
        }
        RespFrame::Integer(value) => {
            dst.put_u8(b':');
            let mut buf = itoa::Buffer::new();
            dst.extend_from_slice(buf.format(*value).as_bytes());
            dst.extend_from_slice(b"\r\n");
        }
        RespFrame::BulkString(None) => {
            dst.extend_from_slice(b"$-1\r\n");
        }
        RespFrame::BulkString(Some(data)) => {
            dst.put_u8(b'$');
            dst.extend_from_slice(data.len().to_string().as_bytes());
            dst.extend_from_slice(b"\r\n");
            dst.extend_from_slice(data);
            dst.extend_from_slice(b"\r\n");
        }
        RespFrame::Array(items) => {
            dst.put_u8(b'*');
            dst.extend_from_slice(items.len().to_string().as_bytes());
            dst.extend_from_slice(b"\r\n");
            for item in items {
                encode_frame(item, dst)?;
            }
        }
    }
    Ok(())
}

fn checked_add(lhs: usize, rhs: usize) -> Result<usize, io::Error> {
    lhs.checked_add(rhs)
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "frame too large"))
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let app = WireframeApp::<BincodeSerializer, (), Envelope>::new()?
        .with_codec(RespFrameCodec::new(1024))
        .route(1, std::sync::Arc::new(|_: &Envelope| Box::pin(async {})))?;

    let _ = app;
    Ok(())
}
