//! RESP encoding functions.

use std::io;

use bytes::{BufMut, BytesMut};

use super::frame::RespFrame;

/// Maximum recursion depth for nested RESP arrays.
///
/// This limit prevents stack overflow when encoding programmatically
/// constructed deeply nested arrays.
const MAX_RECURSION_DEPTH: usize = 64;

/// Compute the encoded length of a RESP frame.
pub fn encoded_len(frame: &RespFrame) -> Result<usize, io::Error> { encoded_len_at(frame, 0) }

/// Measure a frame recursively while enforcing the nesting safety limit.
/// Checked arithmetic turns a size overflow into an input error before the
/// encoder reserves or writes any destination capacity.
fn encoded_len_at(frame: &RespFrame, depth: usize) -> Result<usize, io::Error> {
    if depth > MAX_RECURSION_DEPTH {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "maximum recursion depth exceeded",
        ));
    }
    match frame {
        RespFrame::SimpleString(text) | RespFrame::Error(text) => {
            checked_add(1, checked_add(text.len(), 2)?)
        }
        RespFrame::Integer(value) => {
            let digits = digits_len_i64(*value);
            checked_add(1, checked_add(digits, 2)?)
        }
        RespFrame::BulkString(None) | RespFrame::Array(None) => Ok(5), // $-1 or *-1
        RespFrame::BulkString(Some(data)) => {
            let len_digits = digits_len_usize(data.len());
            let head = checked_add(1, checked_add(len_digits, 2)?)?;
            let body = checked_add(data.len(), 2)?;
            checked_add(head, body)
        }
        RespFrame::Array(Some(items)) => {
            let count_digits = digits_len_usize(items.len());
            let mut total = checked_add(1, checked_add(count_digits, 2)?)?;
            for item in items {
                total = checked_add(total, encoded_len_at(item, depth + 1)?)?;
            }
            Ok(total)
        }
    }
}

/// Encode a RESP frame into the destination buffer.
pub fn encode_frame(frame: &RespFrame, dst: &mut BytesMut) -> Result<(), io::Error> {
    encode_frame_at(frame, dst, 0)
}

/// Append one frame and its nested children using RESP wire ordering.
/// The depth guard matches [`encoded_len_at`] so length validation and actual
/// emission reject the same recursively constructed input.
fn encode_frame_at(frame: &RespFrame, dst: &mut BytesMut, depth: usize) -> Result<(), io::Error> {
    if depth > MAX_RECURSION_DEPTH {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "maximum recursion depth exceeded",
        ));
    }
    match frame {
        RespFrame::SimpleString(text) => {
            dst.put_u8(b'+');
            dst.extend_from_slice(text.as_bytes());
            dst.extend_from_slice(b"\r\n");
        }
        RespFrame::Error(text) => {
            dst.put_u8(b'-');
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
            let mut buf = itoa::Buffer::new();
            dst.extend_from_slice(buf.format(data.len()).as_bytes());
            dst.extend_from_slice(b"\r\n");
            dst.extend_from_slice(data);
            dst.extend_from_slice(b"\r\n");
        }
        RespFrame::Array(None) => {
            dst.extend_from_slice(b"*-1\r\n");
        }
        RespFrame::Array(Some(items)) => {
            dst.put_u8(b'*');
            let mut buf = itoa::Buffer::new();
            dst.extend_from_slice(buf.format(items.len()).as_bytes());
            dst.extend_from_slice(b"\r\n");
            for item in items {
                encode_frame_at(item, dst, depth + 1)?;
            }
        }
    }
    Ok(())
}

/// Count decimal digits without allocating a temporary representation.
fn digits_len_usize(mut value: usize) -> usize {
    let mut digits = 1;
    while value >= 10 {
        value /= 10;
        digits += 1;
    }
    digits
}

/// Count digits in a signed integer, including its possible minus sign.
fn digits_len_i64(value: i64) -> usize {
    let mut digits = 1;
    let mut cursor = value;
    while cursor <= -10 || cursor >= 10 {
        cursor /= 10;
        digits += 1;
    }
    if value < 0 {
        digits + 1
    } else {
        digits
    }
}

/// Add encoded-size components and report overflow as a frame-size error.
fn checked_add(lhs: usize, rhs: usize) -> Result<usize, io::Error> {
    lhs.checked_add(rhs)
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "frame too large"))
}
